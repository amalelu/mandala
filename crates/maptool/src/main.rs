use baumhard::mindmap::loader::load_from_file;
use baumhard::mindmap::model::MindMap;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
Usage: maptool <command> <map.json> <args...>

Commands:
  show <map.json> <node-id>     Print the text of the node with this ID.
  grep <map.json> <pattern>     Print every node whose text contains
                                <pattern> (case-sensitive substring).
                                Use -i before the map path for case-insensitive.";

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
            let mut rest = &args[1..];
            let mut case_insensitive = false;
            if rest.first().map(|s| s.as_str()) == Some("-i") {
                case_insensitive = true;
                rest = &rest[1..];
            }
            let map_path = rest
                .first()
                .ok_or_else(|| CliError::Usage("grep: missing <map.json>".into()))?;
            let pattern = rest
                .get(1)
                .ok_or_else(|| CliError::Usage("grep: missing <pattern>".into()))?;
            let map = load_map(map_path)?;
            let matches = grep_nodes(&map, pattern, case_insensitive);
            if matches.is_empty() {
                return Err(CliError::NotFound(format!("no matches for: {pattern}")));
            }
            for (id, text) in matches {
                print_match(id, text);
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

/// Return every (id, text) pair whose text contains `pattern`.
/// Results are sorted by node ID so output is deterministic.
fn grep_nodes<'a>(
    map: &'a MindMap,
    pattern: &str,
    case_insensitive: bool,
) -> Vec<(&'a str, &'a str)> {
    let needle = if case_insensitive {
        pattern.to_lowercase()
    } else {
        pattern.to_string()
    };
    let mut out: Vec<(&str, &str)> = map
        .nodes
        .values()
        .filter(|n| {
            if case_insensitive {
                n.text.to_lowercase().contains(&needle)
            } else {
                n.text.contains(&needle)
            }
        })
        .map(|n| (n.id.as_str(), n.text.as_str()))
        .collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

/// Format a single grep match. Single-line text stays on one line;
/// multi-line text gets indented underneath the ID for readability.
fn print_match(id: &str, text: &str) {
    if text.contains('\n') {
        println!("{id}:");
        for line in text.lines() {
            println!("  {line}");
        }
    } else {
        println!("{id}: {text}");
    }
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

    #[test]
    fn grep_finds_known_substring() {
        let map = testament();
        let hits = grep_nodes(&map, "Lord God", false);
        assert!(hits.iter().any(|(id, _)| *id == "348068464"));
    }

    #[test]
    fn grep_case_insensitive_matches() {
        let map = testament();
        let insen = grep_nodes(&map, "lord god", true);
        assert!(insen.iter().any(|(id, _)| *id == "348068464"));
    }

    #[test]
    fn grep_empty_on_no_match() {
        let map = testament();
        assert!(grep_nodes(&map, "xyzzy-no-such-token", false).is_empty());
    }
}
