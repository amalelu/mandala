// SPDX-License-Identifier: MPL-2.0

use baumhard::mindmap::loader::{load_from_file, save_to_file};
use baumhard::mindmap::model::MindMap;
use regex::{Regex, RegexBuilder};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

mod convert;
mod export;
mod verify;

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
                                map back in place atomically (temp
                                file + rename). --dry-run skips the
                                write but still invokes <cmd> for each
                                matched node, so commands with side
                                effects will still execute. Zero
                                matches is an error (exit 1), matching
                                `grep`.
  export <map.json> [out.md]    Render the node tree as a Markdown
                                document. The first line of each
                                node's text becomes a heading whose
                                depth matches the node's generation
                                (#, ##, ###, ...); any further lines
                                appear as plain text under it.
                                Empty-text nodes are transparent —
                                their children surface at the same
                                depth. Notes, fonts, and edges are
                                ignored. Writes to stdout, or to
                                <out.md> if a second path is given.
  convert --legacy <in.json> <out.json>
                                Convert a legacy (miMind-derived) map
                                to the current format: structural IDs,
                                named enums, hoisted palettes, channel
                                field, legacy portals folded into
                                portal-mode edges, and node text folded
                                into a sections[] array. One hop — the
                                output loads and verifies without a
                                follow-up convert.
  convert --portals <in.json> <out.json>
                                Migrate a pre-refactor map whose
                                portals live in a top-level portals
                                array to the unified form (portals
                                are edges with display_mode portal).
                                Also runs inside --legacy; use this
                                verb for a map that is otherwise
                                already current.
  convert --sections <in.json> <out.json>
                                Migrate a pre-section-refactor map
                                whose nodes carry text / text_runs
                                directly into the post-refactor shape
                                where each node has a sections[] array.
                                Each legacy node folds into a single
                                default section; idempotent on already-
                                migrated maps. Also runs inside
                                --legacy.

                                All three convert verbs accept the same
                                path for <in.json> and <out.json>: the
                                read completes first, and the write
                                stages to a temp file and renames, so
                                an interrupted run leaves the original
                                intact rather than truncated.
  verify <map.json>             Check the file against the format's
                                structural invariants (parent_id
                                consistency, Dewey IDs, edge and portal
                                references, palette references, named
                                enums, text-run bounds). Exit 0 if
                                valid; nonzero with a list of
                                violations otherwise.";

fn main() -> ExitCode {
    // maptool is a shipped release binary that walks the same
    // baumhard load/save paths the app does, so it hits the same §9
    // degrade sites — `loader`'s duplicate-edge warn, the model's
    // validation errors. Without an installed logger those go to
    // `log`'s no-op default and the tool reports a clean run over a
    // map it silently repaired.
    baumhard::util::log::init();

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

/// CLI exit-code dispatch. Distinct failure modes need distinct exit
/// codes; §9's "no custom error types" rule targets the interactive
/// app posture, not a scriptable tool.
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
            let text = map
                .nodes
                .get(node_id)
                .map(|n| n.display_text())
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
                return Err(CliError::NotFound(format!("no matches for: {}", parsed.pattern)));
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
            let targets = select_section_targets(&map, &regex, parsed.target_notes);
            if targets.is_empty() {
                return Err(CliError::NotFound(format!(
                    "no nodes matched: {}",
                    parsed.pattern
                )));
            }
            let changed = apply_command(
                &mut map,
                &targets,
                parsed.target_notes,
                parsed.cmd,
                parsed.cmd_args,
            )?;
            if parsed.dry_run {
                eprintln!("dry-run: would modify {} target(s):", changed.len());
                for (id, section_idx) in &changed {
                    if parsed.target_notes {
                        eprintln!("  {id} (notes)");
                    } else {
                        eprintln!("  {id}[{section_idx}]");
                    }
                }
            } else if !changed.is_empty() {
                save_to_file(Path::new(parsed.map_path), &map).map_err(CliError::Io)?;
            }
            Ok(())
        }
        "export" => {
            let map_path = args
                .get(1)
                .ok_or_else(|| CliError::Usage("export: missing <map.json>".into()))?;
            let out_path = args.get(2);
            let map = load_map(map_path)?;
            let markdown = export::mindmap_to_markdown(&map);
            match out_path {
                None => {
                    print!("{markdown}");
                    Ok(())
                }
                Some(path) => fs::write(Path::new(path), &markdown)
                    .map_err(|e| CliError::Io(format!("failed to write {path}: {e}"))),
            }
        }
        "convert" => match args.get(1).map(|s| s.as_str()) {
            Some("--legacy") => {
                let input = args
                    .get(2)
                    .ok_or_else(|| CliError::Usage("convert: missing <in.json>".into()))?;
                let output = args
                    .get(3)
                    .ok_or_else(|| CliError::Usage("convert: missing <out.json>".into()))?;
                convert::convert_legacy(Path::new(input), Path::new(output)).map_err(CliError::Io)
            }
            Some("--portals") => {
                let input = args
                    .get(2)
                    .ok_or_else(|| CliError::Usage("convert: missing <in.json>".into()))?;
                let output = args
                    .get(3)
                    .ok_or_else(|| CliError::Usage("convert: missing <out.json>".into()))?;
                convert::convert_portals(Path::new(input), Path::new(output)).map_err(CliError::Io)
            }
            Some("--sections") => {
                let input = args
                    .get(2)
                    .ok_or_else(|| CliError::Usage("convert: missing <in.json>".into()))?;
                let output = args
                    .get(3)
                    .ok_or_else(|| CliError::Usage("convert: missing <out.json>".into()))?;
                convert::convert_sections(Path::new(input), Path::new(output)).map_err(CliError::Io)
            }
            _ => Err(CliError::Usage(
                "convert: expected --legacy, --portals, or --sections flag".into(),
            )),
        },
        "verify" => {
            let map_path = args
                .get(1)
                .ok_or_else(|| CliError::Usage("verify: missing <map.json>".into()))?;
            let map = load_map(map_path)?;
            let violations = verify::verify(&map);
            if violations.is_empty() {
                println!("{}: valid", map_path);
                Ok(())
            } else {
                let mut errors = 0usize;
                let mut warnings = 0usize;
                for v in &violations {
                    match v.severity {
                        verify::Severity::Warning => {
                            warnings += 1;
                            eprintln!("warning: {v}");
                        }
                        verify::Severity::Error => {
                            errors += 1;
                            eprintln!("{v}");
                        }
                    }
                }
                if errors == 0 {
                    eprintln!("{warnings} warning(s)",);
                    Ok(())
                } else {
                    eprintln!("{} violation(s)", violations.len());
                    Err(CliError::NotFound(format!(
                        "{} violation(s) in {}",
                        violations.len(),
                        map_path
                    )))
                }
            }
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

/// Parsed positional args for `grep`.
struct GrepArgs<'a> {
    map_path: &'a str,
    pattern: &'a str,
    case_insensitive: bool,
}

/// Parse args after `grep`. `-i` is position-independent; everything
/// else is positional in declared order.
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

/// Compile `pattern` into a regex. The error message is unprefixed —
/// callers add the subcommand name.
fn build_regex(pattern: &str, case_insensitive: bool) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|e| format!("invalid regex {pattern:?}: {e}"))
}

/// Return `(id, line)` for every line of `text` (across every
/// section) or `notes` matching `regex`. Sort: numeric-id-first
/// when both parse as `u64`, lexicographic otherwise; stable, so
/// section text lines precede `notes` lines for a single node and
/// section ordering is preserved within a node.
fn grep_nodes<'a>(map: &'a MindMap, regex: &Regex) -> Vec<(&'a str, &'a str)> {
    let mut out: Vec<(&'a str, &'a str)> = Vec::new();
    for node in map.nodes.values() {
        for section in &node.sections {
            for line in section.text.lines() {
                if regex.is_match(line) {
                    out.push((node.id.as_str(), line));
                }
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
/// `--notes`, and `--dry-run` are recognized anywhere before the `--`
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
            // Reject unknown `--` flags so typos like `--dry-runn` don't
            // get silently swallowed as a positional arg.
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("apply: unknown flag: {other}")));
            }
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
    let sep_at =
        sep_at.ok_or_else(|| CliError::Usage("apply: missing `--` separator before command".into()))?;
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

/// Sorted `(node_id, section_idx)` tuples for sections whose text
/// matches `regex` (when `target_notes` is false), or `(node_id, 0)`
/// for nodes whose `notes` match. Section-aware: a multi-section
/// node where only `sections[1]` matches yields `(id, 1)` so the
/// apply path writes to that section, not the first one. Pre-fix
/// the function returned only `node_id`s and `apply_command`
/// hard-coded `sections[0]` — silent data corruption when a
/// multi-section node had a match outside section 0.
///
/// `notes` matches collapse to `(id, 0)` since the section index
/// is irrelevant to a notes-targeted apply; the apply path
/// branches on `target_notes` and ignores the index in that case.
///
/// Sort: numeric on `node_id` then `section_idx` for stable
/// output across runs.
fn select_section_targets(map: &MindMap, regex: &Regex, target_notes: bool) -> Vec<(String, usize)> {
    let mut targets: Vec<(String, usize)> = Vec::new();
    for node in map.nodes.values() {
        if target_notes {
            if node.notes.lines().any(|line| regex.is_match(line)) {
                targets.push((node.id.clone(), 0));
            }
        } else {
            for (idx, section) in node.sections.iter().enumerate() {
                if section.text.lines().any(|line| regex.is_match(line)) {
                    targets.push((node.id.clone(), idx));
                }
            }
        }
    }
    targets.sort_by(|a, b| match (a.0.parse::<u64>(), b.0.parse::<u64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y).then(a.1.cmp(&b.1)),
        _ => a.0.cmp(&b.0).then(a.1.cmp(&b.1)),
    });
    targets
}

/// Sorted unique node IDs from a list of `(node_id, section_idx)`
/// targets. Currently only used by tests to assert the node-level
/// set without restating the section_idx (the production paths in
/// `run` use the full target list).
#[cfg(test)]
fn unique_node_ids(targets: &[(String, usize)]) -> Vec<String> {
    let mut ids: Vec<String> = targets.iter().map(|(id, _)| id.clone()).collect();
    ids.dedup();
    ids
}

/// For each `(node_id, section_idx)` target in `targets`, pipe the
/// target field through `cmd` and replace the field with the
/// command's stdout. When `target_notes` is false, the apply path
/// operates on the **matched section** (not hard-coded
/// `sections[0]` as before): its text is the input, the result
/// is written back to that section's `text`, and the section's
/// `text_runs` are cleared (byte offsets would otherwise point
/// into stale positions). Multi-section nodes route correctly to
/// the section the regex matched.
///
/// When `target_notes` is true, `notes` is the target field and
/// section state is left alone; `section_idx` is ignored.
///
/// Returns the list of `(id, section_idx)` whose target field was
/// actually modified, preserving the input order. Aborts on the
/// first subprocess failure without touching subsequent targets —
/// callers that then choose not to save get all-or-nothing
/// semantics.
fn apply_command(
    map: &mut MindMap,
    targets: &[(String, usize)],
    target_notes: bool,
    cmd: &str,
    cmd_args: &[String],
) -> Result<Vec<(String, usize)>, CliError> {
    let mut changed: Vec<(String, usize)> = Vec::new();
    for (id, section_idx) in targets {
        let node = map
            .nodes
            .get_mut(id)
            .expect("id came from select_section_targets, must exist in map");
        let input = if target_notes {
            node.notes.clone()
        } else {
            node.sections
                .get(*section_idx)
                .map(|s| s.text.clone())
                .unwrap_or_default()
        };
        let new_value = run_pipe(cmd, cmd_args, &input)?;
        if new_value != input {
            if target_notes {
                node.notes = new_value;
            } else if let Some(section) = node.sections.get_mut(*section_idx) {
                section.text = new_value;
                section.text_runs.clear();
            }
            changed.push((id.clone(), *section_idx));
        }
    }
    Ok(changed)
}

/// Spawn `cmd cmd_args`, pipe `input` to its stdin from a writer
/// thread (so payloads larger than the pipe buffer don't deadlock),
/// and return stdout. Strips one trailing `\n` or `\r\n`. Non-zero
/// exit becomes `CliError::Subprocess(stderr)`; EPIPE on stdin is
/// swallowed so the child's real status surfaces.
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
    let input_bytes = input.as_bytes().to_vec();
    let cmd_name = cmd.to_string();
    let writer = std::thread::spawn(move || -> Result<(), String> {
        if let Err(e) = stdin.write_all(&input_bytes) {
            if e.kind() != std::io::ErrorKind::BrokenPipe {
                return Err(format!("`{cmd_name}`: write stdin: {e}"));
            }
        }
        drop(stdin); // close the pipe so the child sees EOF
        Ok(())
    });
    let output = child
        .wait_with_output()
        .map_err(|e| CliError::Subprocess(format!("`{cmd}`: wait: {e}")))?;
    match writer.join() {
        Ok(Ok(())) => {}
        Ok(Err(msg)) => return Err(CliError::Subprocess(msg)),
        Err(_) => {
            return Err(CliError::Subprocess(format!(
                "`{cmd}`: stdin writer thread panicked"
            )))
        }
    }
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
    let mut out = String::from_utf8(output.stdout)
        .map_err(|e| CliError::Subprocess(format!("`{cmd}` produced non-UTF-8 output: {e}")))?;
    if out.ends_with('\n') {
        out.pop();
        if out.ends_with('\r') {
            out.pop();
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::util::test_temp::TempDir;
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

    // --- grep / grep_nodes ------------------------------------------

    #[test]
    fn grep_finds_literal_pattern() {
        let map = testament();
        let hits = grep_nodes(&map, &rx("Lord God", false));
        assert!(hits.iter().any(|(id, _)| *id == "0"));
    }

    #[test]
    fn grep_case_insensitive_matches() {
        let map = testament();
        let insen = grep_nodes(&map, &rx("lord god", true));
        assert!(insen.iter().any(|(id, _)| *id == "0"));
    }

    #[test]
    fn grep_empty_on_no_match() {
        let map = testament();
        assert!(grep_nodes(&map, &rx("xyzzy-no-such-token", false)).is_empty());
    }

    #[test]
    fn grep_invalid_regex_message() {
        // build_regex returns the message unprefixed (caller adds "grep:").
        let err = build_regex("[unclosed", false).unwrap_err();
        assert!(err.contains("invalid regex"), "got: {err}");
        assert!(
            !err.starts_with("grep:"),
            "build_regex must not hardcode subcommand prefix"
        );
    }

    #[test]
    fn grep_searches_notes_field() {
        // Inject a unique sentinel into one node's notes. No other
        // node in testament contains this token, and it isn't in
        // any node's text — so finding it proves notes are searched.
        let mut map = testament();
        map.nodes.get_mut("0").unwrap().notes = "SENTINEL_ZXCVBNM_12345".into();

        let hits = grep_nodes(&map, &rx("SENTINEL_ZXCVBNM_12345", false));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "0");
        assert!(hits[0].1.contains("SENTINEL_ZXCVBNM_12345"));
    }

    #[test]
    fn grep_returns_text_lines_before_notes_lines() {
        let mut map = testament();
        let node = map.nodes.get_mut("0").unwrap();
        node.sections[0].text = "MARK_A\nMARK_B".into();
        node.sections[0].text_runs.clear();
        node.notes = "MARK_C".into();

        let hits = grep_nodes(&map, &rx("^MARK_", false));
        let just_this: Vec<&str> = hits
            .iter()
            .filter(|(id, _)| *id == "0")
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
        // -i between map and pattern must still be recognized — the
        // parser treats `-i` as position-independent.
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

    #[test]
    fn run_verify_on_testament_succeeds() {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.pop();
        p.pop();
        p.push("maps/testament.mindmap.json");
        let args = as_strings(&["verify", p.to_str().unwrap()]);
        assert!(run(&args).is_ok(), "testament map must verify clean");
    }

    // --- convert --legacy: the one-hop migration contract ------------
    //
    // `format/migration.md` promises "a single legacy hop produces a
    // post-section file in one step" and that `maptool verify` on the
    // output "should exit 0". A legacy map that carried portals broke
    // both: the fold never ran, the top-level `portals[]` survived,
    // and the loader refused the converted file — so `verify` could
    // not even parse it. These tests are the end-to-end pin.

    fn legacy_with_portals_path() -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures/legacy_with_portals.mindmap.json");
        p
    }

    /// Guards the fixture itself: if a future edit made it loadable,
    /// the round-trip test below would pass without proving anything.
    #[test]
    fn test_legacy_with_portals_fixture_is_rejected_by_the_loader() {
        let err = load_from_file(&legacy_with_portals_path())
            .expect_err("the legacy fixture must not load as a current-format map");
        assert!(
            err.contains("portals"),
            "fixture should trip the legacy-portals detector, got: {err}"
        );
    }

    #[test]
    fn test_run_convert_legacy_on_portal_bearing_map_loads_and_verifies_in_one_hop() {
        let dir = TempDir::new("convert-legacy-portals");
        let output = dir.join("converted.mindmap.json");
        let args = as_strings(&[
            "convert",
            "--legacy",
            legacy_with_portals_path().to_str().unwrap(),
            output.to_str().unwrap(),
        ]);
        run(&args).expect("convert --legacy must succeed");

        // The loader is the real acceptance test: pre-fix it rejected
        // this file, pointing at `maptool convert --portals`.
        let map = load_from_file(&output).expect("converted map must load");

        let portal_edges: Vec<_> = map
            .edges
            .iter()
            .filter(|e| e.display_mode.as_deref() == Some("portal"))
            .collect();
        assert_eq!(portal_edges.len(), 1, "the legacy portal must survive as an edge");
        let portal = portal_edges[0];
        // Endpoints carry the *new* Dewey ids, so the fold has to run
        // after the id rewrite, not before it.
        assert_eq!(portal.from_id, "0.0");
        assert_eq!(portal.to_id, "0.1");
        assert_eq!(portal.edge_type, "cross_link");
        assert_eq!(portal.color, "#ff00aa");
        let glyph = portal
            .glyph_connection
            .as_ref()
            .expect("portal edge must carry its marker glyph");
        assert_eq!(glyph.body, "⬢");
        assert_eq!(glyph.font.as_deref(), Some("LiberationSans"));
        assert_eq!(glyph.font_size_pt, 18.0);

        // The rest of the legacy hop must still land: integer enums
        // named, node text folded into sections, palettes hoisted.
        assert_eq!(map.nodes["0"].sections[0].text, "Legacy root");
        assert_eq!(map.nodes["0"].style.shape, "rounded_rectangle");
        assert_eq!(map.nodes["0.1"].layout.layout_type, "outline");
        assert!(map.palettes.contains_key("coral-v3"));

        // ...and `verify` exits 0 on the output, in that one hop.
        let verify_args = as_strings(&["verify", output.to_str().unwrap()]);
        assert!(
            run(&verify_args).is_ok(),
            "converted map must verify clean without a follow-up convert"
        );
    }

    /// Running the standalone portal verb over an already-folded map
    /// is a no-op, so the two paths compose instead of double-folding.
    #[test]
    fn test_run_convert_portals_after_legacy_is_a_no_op() {
        let dir = TempDir::new("convert-legacy-then-portals");
        let output = dir.join("converted.mindmap.json");
        let legacy = as_strings(&[
            "convert",
            "--legacy",
            legacy_with_portals_path().to_str().unwrap(),
            output.to_str().unwrap(),
        ]);
        run(&legacy).unwrap();
        let once = fs::read_to_string(&output).unwrap();

        let portals = as_strings(&[
            "convert",
            "--portals",
            output.to_str().unwrap(),
            output.to_str().unwrap(),
        ]);
        run(&portals).unwrap();
        let twice = fs::read_to_string(&output).unwrap();

        // Parsed trees first: when something *has* drifted, this
        // failure names the key instead of printing two screens of
        // near-identical JSON (TEST_CONVENTIONS §T5 — improve the
        // values being compared, not the macro).
        let before: serde_json::Value = serde_json::from_str(&once).unwrap();
        let after: serde_json::Value = serde_json::from_str(&twice).unwrap();
        assert_eq!(
            after, before,
            "convert --portals must not change an already-folded map"
        );
        // ...then byte equality, which is the stronger claim and the
        // one `convert/sections.rs` leans on when it keeps converted
        // maps "byte-stable". Equal trees can still differ in key
        // order or number rendering; a second hop must reproduce the
        // identical file, not merely an equivalent one.
        assert_eq!(
            convert::content_digest(&twice),
            convert::content_digest(&once),
            "a second convert --portals hop must be byte-identical, not just equivalent"
        );
    }

    #[test]
    fn run_verify_flags_invalid_fixture() {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures/invalid_sampler.mindmap.json");
        let args = as_strings(&["verify", p.to_str().unwrap()]);
        match run(&args) {
            Err(CliError::NotFound(msg)) => {
                assert!(msg.contains("violation"), "got: {msg}");
            }
            other => panic!("expected NotFound with violations, got {other:?}"),
        }
    }

    #[test]
    fn run_verify_missing_map_is_usage_error() {
        let args = as_strings(&["verify"]);
        assert!(matches!(run(&args), Err(CliError::Usage(_))));
    }

    #[test]
    fn run_verify_warns_but_succeeds_on_broadcast_channel() {
        use baumhard::mindmap::model::MindSection;
        let mut map = MindMap::new_blank("broadcast");
        let mut n = baumhard::mindmap::model::MindNode {
            id: "0".into(),
            parent_id: None,
            position: baumhard::mindmap::model::Position { x: 0.0, y: 0.0 },
            size: baumhard::mindmap::model::Size {
                width: 100.0,
                height: 40.0,
            },
            sections: vec![
                {
                    let mut s = MindSection::new_default("a".into(), Vec::new());
                    s.channel = Some(1);
                    s
                },
                {
                    let mut s = MindSection::new_default("b".into(), Vec::new());
                    s.channel = Some(1);
                    s
                },
            ],
            style: baumhard::mindmap::model::NodeStyle {
                background_color: "#000000".into(),
                frame_color: "#ffffff".into(),
                text_color: "#ffffff".into(),
                shape: "rectangle".into(),
                corner_radius_percent: 0.0,
                frame_thickness: 0.0,
                show_frame: false,
                show_shadow: false,
                border: None,
            },
            layout: baumhard::mindmap::model::NodeLayout {
                layout_type: "map".into(),
                direction: "auto".into(),
                spacing: 0.0,
            },
            folded: false,
            notes: String::new(),
            color_schema: None,
            channel: 0,
            trigger_bindings: Vec::new(),
            inline_mutations: Vec::new(),
            inline_macros: Vec::new(),
            min_zoom_to_render: None,
            max_zoom_to_render: None,
        };
        n.sections[0].offset = baumhard::mindmap::model::Position { x: 0.0, y: 0.0 };
        n.sections[0].size = Some(baumhard::mindmap::model::Size {
            width: 10.0,
            height: 10.0,
        });
        n.sections[1].offset = baumhard::mindmap::model::Position { x: 0.0, y: 10.0 };
        n.sections[1].size = Some(baumhard::mindmap::model::Size {
            width: 10.0,
            height: 10.0,
        });
        map.nodes.insert("0".into(), n);

        let scratch = TempDir::new("verify-broadcast-channel");
        let path = scratch.join("map.mindmap.json");
        save_to_file(&path, &map).unwrap();
        let args = as_strings(&["verify", path.to_str().unwrap()]);
        assert!(run(&args).is_ok(), "warning-only verify must exit 0");
    }

    // --- apply: fixture + tmpfile helpers ---------------------------
    //
    // Apply tests use a hand-crafted fixture so assertions can name
    // every node by ID. `TmpMap` copies it per-test for parallel safety.

    fn apply_fixture_path() -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures/apply_test.mindmap.json");
        p
    }

    fn apply_fixture() -> MindMap {
        load_from_file(&apply_fixture_path()).unwrap()
    }

    /// RAII guard for a per-test copy of the apply fixture, living in
    /// its own [`TempDir`] so parallel runs cannot collide and a panic
    /// mid-test still cleans up. The directory is held (not just the
    /// path) because dropping it is what removes the copy; field order
    /// is irrelevant here since `remove_dir_all` takes the file with
    /// it either way.
    struct TmpMap {
        _dir: TempDir,
        path: PathBuf,
    }

    impl TmpMap {
        fn new(name: &str) -> Self {
            let dir = TempDir::new(&format!("apply-{name}"));
            let path = dir.join("map.mindmap.json");
            std::fs::copy(apply_fixture_path(), &path).unwrap();
            Self { _dir: dir, path }
        }
        fn path(&self) -> &Path {
            &self.path
        }
    }

    // --- select_section_targets ------------------------------------

    #[test]
    fn select_section_targets_text_field_matches_hello() {
        let map = apply_fixture();
        let targets = select_section_targets(&map, &rx("hello", false), false);
        assert_eq!(
            unique_node_ids(&targets),
            vec!["0".to_string(), "0.2".to_string()]
        );
    }

    #[test]
    fn select_section_targets_text_field_ignores_notes() {
        let map = apply_fixture();
        // NOTES_TOKEN only appears in n2's notes field, not any text.
        let targets = select_section_targets(&map, &rx("NOTES_TOKEN", false), false);
        assert!(targets.is_empty(), "text-target should ignore notes: {targets:?}");
    }

    #[test]
    fn select_section_targets_notes_field_matches_only_notes() {
        let map = apply_fixture();
        let targets = select_section_targets(&map, &rx("NOTES_TOKEN", false), true);
        assert_eq!(targets, vec![("0.0".to_string(), 0)]);
    }

    #[test]
    fn select_section_targets_case_insensitive() {
        let map = apply_fixture();
        let targets = select_section_targets(&map, &rx("HELLO", true), false);
        assert_eq!(
            unique_node_ids(&targets),
            vec!["0".to_string(), "0.2".to_string()]
        );
    }

    #[test]
    fn select_section_targets_no_match_empty() {
        let map = apply_fixture();
        assert!(select_section_targets(&map, &rx("xyzzy_absent", false), false).is_empty());
    }

    #[test]
    fn select_section_targets_routes_to_matched_section_index() {
        // Pre-fix `apply` always wrote `sections[0]` regardless of
        // which section matched — silent data corruption on
        // multi-section nodes. Pin the section_idx propagation:
        // a node with `sections[0]="alpha"`, `sections[1]="match"`
        // surfaces `(id, 1)`, not `(id, 0)`.
        use baumhard::mindmap::model::MindSection;
        let mut map = apply_fixture();
        map.nodes
            .get_mut("0.1")
            .unwrap()
            .sections
            .push(MindSection::new_default(
                "hello-from-section-1".into(),
                Vec::new(),
            ));
        let targets = select_section_targets(&map, &rx("hello-from-section-1", false), false);
        assert_eq!(targets, vec![("0.1".to_string(), 1)]);
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
        let out = run_pipe("sh", &["-c".into(), "cat; echo".into()], "abc").unwrap();
        assert_eq!(out, "abc", "exactly one trailing newline should be stripped");
    }

    #[test]
    fn run_pipe_strips_only_one_newline() {
        // Two `echo`s emit two trailing newlines; only one is stripped.
        let out = run_pipe("sh", &["-c".into(), "cat; echo; echo".into()], "abc").unwrap();
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
        let targets = vec![("0".to_string(), 0), ("0.2".to_string(), 0)];
        let changed = apply_command(&mut map, &targets, false, "tr", &["a-z".into(), "A-Z".into()]).unwrap();
        assert_eq!(changed, vec![("0".to_string(), 0), ("0.2".to_string(), 0)]);
        assert_eq!(map.nodes["0"].sections[0].text, "HELLO WORLD");
        assert!(
            map.nodes["0"].sections[0].text_runs.is_empty(),
            "text_runs should be cleared when text changes"
        );
        assert_eq!(map.nodes["0.2"].sections[0].text, "HELLO AGAIN");
        assert!(map.nodes["0.2"].sections[0].text_runs.is_empty());
        // Untouched node keeps its runs.
        assert_eq!(map.nodes["0.0"].sections[0].text, "Alpha\nBeta\nGamma");
        assert_eq!(map.nodes["0.0"].sections[0].text_runs.len(), 1);
    }

    #[test]
    fn apply_command_writes_to_matched_section_not_section_zero() {
        // Pre-fix the apply path hard-coded `sections[0]`. Pin the
        // critical: a `(node_id, 1)` target writes to section 1,
        // leaving section 0 untouched.
        use baumhard::mindmap::model::MindSection;
        let mut map = apply_fixture();
        map.nodes
            .get_mut("0.1")
            .unwrap()
            .sections
            .push(MindSection::new_default("section-one-text".into(), Vec::new()));
        let targets = vec![("0.1".to_string(), 1)];
        let changed = apply_command(&mut map, &targets, false, "tr", &["a-z".into(), "A-Z".into()]).unwrap();
        assert_eq!(changed, vec![("0.1".to_string(), 1)]);
        assert_eq!(
            map.nodes["0.1"].sections[0].text, "unchanged",
            "section 0 must not be touched when target is section 1"
        );
        assert_eq!(map.nodes["0.1"].sections[1].text, "SECTION-ONE-TEXT");
    }

    #[test]
    fn apply_command_notes_preserves_text_and_runs() {
        let mut map = apply_fixture();
        let original_text = map.nodes["0.0"].sections[0].text.clone();
        let before_len = map.nodes["0.0"].sections[0].text_runs.len();
        let before_start = map.nodes["0.0"].sections[0].text_runs[0].start;
        let before_end = map.nodes["0.0"].sections[0].text_runs[0].end;
        let targets = vec![("0.0".to_string(), 0)];
        let changed = apply_command(&mut map, &targets, true, "tr", &["a-z".into(), "A-Z".into()]).unwrap();
        assert_eq!(changed, vec![("0.0".to_string(), 0)]);
        assert_eq!(map.nodes["0.0"].notes, "SECRET NOTES_TOKEN HERE");
        assert_eq!(map.nodes["0.0"].sections[0].text, original_text, "text untouched");
        assert_eq!(map.nodes["0.0"].sections[0].text_runs.len(), before_len);
        assert_eq!(map.nodes["0.0"].sections[0].text_runs[0].start, before_start);
        assert_eq!(map.nodes["0.0"].sections[0].text_runs[0].end, before_end);
    }

    #[test]
    fn apply_command_idempotent_when_output_equals_input() {
        let mut map = apply_fixture();
        let targets = vec![("0.1".to_string(), 0)];
        let changed = apply_command(&mut map, &targets, false, "cat", &[]).unwrap();
        assert!(changed.is_empty(), "expected no change, got: {changed:?}");
        assert_eq!(map.nodes["0.1"].sections[0].text, "unchanged");
    }

    #[test]
    fn apply_command_subprocess_failure_propagates() {
        let mut map = apply_fixture();
        let targets = vec![("0".to_string(), 0)];
        let result = apply_command(&mut map, &targets, false, "sh", &["-c".into(), "exit 4".into()]);
        assert!(matches!(result, Err(CliError::Subprocess(_))));
    }

    // --- run() dispatch for apply -----------------------------------

    #[test]
    fn run_apply_end_to_end_text() {
        let tmp = TmpMap::new("end_to_end_text");
        let args = as_strings(&[
            "apply",
            tmp.path().to_str().unwrap(),
            "hello",
            "--",
            "tr",
            "a-z",
            "A-Z",
        ]);
        assert!(run(&args).is_ok());
        let reloaded = load_from_file(tmp.path()).unwrap();
        assert_eq!(reloaded.nodes["0"].sections[0].text, "HELLO WORLD");
        assert_eq!(reloaded.nodes["0.2"].sections[0].text, "HELLO AGAIN");
        assert!(reloaded.nodes["0"].sections[0].text_runs.is_empty());
        assert!(reloaded.nodes["0.2"].sections[0].text_runs.is_empty());
        // Nodes that didn't match keep their content and their runs.
        assert_eq!(reloaded.nodes["0.0"].sections[0].text, "Alpha\nBeta\nGamma");
        assert_eq!(reloaded.nodes["0.0"].sections[0].text_runs.len(), 1);
        assert_eq!(reloaded.nodes["0.1"].sections[0].text, "unchanged");
    }

    #[test]
    fn run_apply_end_to_end_notes() {
        let tmp = TmpMap::new("end_to_end_notes");
        let args = as_strings(&[
            "apply",
            tmp.path().to_str().unwrap(),
            "NOTES_TOKEN",
            "--notes",
            "--",
            "tr",
            "a-z",
            "A-Z",
        ]);
        assert!(run(&args).is_ok());
        let reloaded = load_from_file(tmp.path()).unwrap();
        assert_eq!(reloaded.nodes["0.0"].notes, "SECRET NOTES_TOKEN HERE");
        assert_eq!(reloaded.nodes["0.0"].sections[0].text, "Alpha\nBeta\nGamma");
        assert_eq!(
            reloaded.nodes["0.0"].sections[0].text_runs.len(),
            1,
            "--notes edits should leave text_runs alone"
        );
    }

    #[test]
    fn run_apply_dry_run_does_not_modify_file() {
        let tmp = TmpMap::new("dry_run");
        let before = std::fs::read(tmp.path()).unwrap();
        let args = as_strings(&[
            "apply",
            tmp.path().to_str().unwrap(),
            "hello",
            "--dry-run",
            "--",
            "tr",
            "a-z",
            "A-Z",
        ]);
        assert!(run(&args).is_ok());
        let after = std::fs::read(tmp.path()).unwrap();
        assert_eq!(before, after, "--dry-run must not write the map");
    }

    #[test]
    fn run_apply_no_matches_is_not_found_and_no_write() {
        let tmp = TmpMap::new("no_match");
        let before = std::fs::read(tmp.path()).unwrap();
        let args = as_strings(&[
            "apply",
            tmp.path().to_str().unwrap(),
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
        let after = std::fs::read(tmp.path()).unwrap();
        assert_eq!(before, after, "no-match run must not write the map");
    }

    #[test]
    fn run_apply_subprocess_failure_leaves_file_unchanged() {
        let tmp = TmpMap::new("subprocess_fail");
        let before = std::fs::read(tmp.path()).unwrap();
        let args = as_strings(&[
            "apply",
            tmp.path().to_str().unwrap(),
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
        let after = std::fs::read(tmp.path()).unwrap();
        assert_eq!(before, after, "file must be unchanged when any subprocess fails");
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
        let args = as_strings(&["-i", "map.json", "--notes", "--dry-run", "pat", "--", "cmd"]);
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
        let args = as_strings(&["apply", "__does_not_exist.json", "[unclosed", "--", "cat"]);
        match run(&args) {
            Err(CliError::Usage(msg)) => assert!(msg.starts_with("apply: invalid regex")),
            other => panic!("expected apply: invalid regex usage error, got {other:?}"),
        }
    }

    #[test]
    fn parse_apply_args_unknown_long_flag_errors() {
        let args = as_strings(&["map.json", "pat", "--dry-runn", "--", "cat"]);
        match parse_apply_args(&args) {
            Err(CliError::Usage(msg)) => {
                assert!(msg.contains("--dry-runn"), "got: {msg}");
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn parse_apply_args_dash_leading_pattern_is_positional() {
        // `-foo` is a valid regex (literal "-foo"); our strict check
        // only fires for double-dash prefixes. Patterns with a single
        // leading `-` remain usable without escaping.
        let args = as_strings(&["map.json", "-foo", "--", "cat"]);
        let p = parse_apply_args(&args).unwrap();
        assert_eq!(p.map_path, "map.json");
        assert_eq!(p.pattern, "-foo");
    }

    // --- save round-trip on the maptool fixture ---------------------
    //
    // Determinism (sorted keys) and atomicity (no tmp leftover) of the
    // canonical save now live in `baumhard::mindmap::loader::tests` —
    // see `test_save_to_file_is_deterministic` and
    // `test_save_to_file_leaves_no_tmp_file_on_success` there. This
    // file keeps a maptool-specific round-trip on the apply fixture
    // (which exercises text-runs + per-section content) so a
    // serde-shape regression that escapes the typed-baumhard tests
    // still gets caught at the maptool seam.

    #[test]
    fn save_map_roundtrip_preserves_content_on_apply_fixture() {
        let tmp = TmpMap::new("roundtrip");
        let map = apply_fixture();
        save_to_file(tmp.path(), &map).unwrap();
        let back = load_from_file(tmp.path()).unwrap();
        for (id, original) in &map.nodes {
            let reloaded = &back.nodes[id];
            assert_eq!(reloaded.notes, original.notes, "{id}: notes");
            assert_eq!(
                reloaded.sections.len(),
                original.sections.len(),
                "{id}: section count"
            );
            for (s_idx, (orig_s, reloaded_s)) in
                original.sections.iter().zip(reloaded.sections.iter()).enumerate()
            {
                assert_eq!(reloaded_s.text, orig_s.text, "{id}/{s_idx}: text");
                assert_eq!(
                    reloaded_s.text_runs.len(),
                    orig_s.text_runs.len(),
                    "{id}/{s_idx}: runs len"
                );
            }
        }
    }

    // --- run_pipe: deadlock avoidance -------------------------------

    #[test]
    fn run_pipe_handles_input_larger_than_pipe_buffer() {
        // 256 KiB > pipe buffer; deadlocks a sync writer, fine for the
        // threaded one.
        let big = "x".repeat(256 * 1024);
        let out = run_pipe("cat", &[], &big).unwrap();
        assert_eq!(out.len(), big.len());
        assert_eq!(out, big);
    }
}
