//! On-disk console-history persistence. Best-effort: every path
//! swallows IO errors and logs a warning — a failing history file
//! must never take the app down.

use crate::application::console::MAX_HISTORY;

/// Load persisted console history from `$XDG_STATE_HOME/mandala/history`
/// (or `$HOME/.local/state/mandala/history`). Returns an empty vec
/// on any failure — history is best-effort and must never take the
/// app down.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn load_console_history() -> Vec<String> {
    let path = match console_history_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<String> = contents
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();
    if out.len() > MAX_HISTORY {
        let drop = out.len() - MAX_HISTORY;
        out.drain(..drop);
    }
    out
}

/// Write the current history to disk. Best-effort — logs and moves
/// on if the directory can't be created or the file can't be written.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn save_console_history(history: &[String]) {
    let path = match console_history_path() {
        Some(p) => p,
        None => return,
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!("console history: create dir {}: {}", parent.display(), e);
            return;
        }
    }
    let start = history.len().saturating_sub(MAX_HISTORY);
    let body: String = history[start..].join("\n") + "\n";
    if let Err(e) = std::fs::write(&path, body) {
        log::warn!("console history: write {}: {}", path.display(), e);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn console_history_path() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        if !xdg.is_empty() {
            let mut p = PathBuf::from(xdg);
            p.push("mandala");
            p.push("history");
            return Some(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            let mut p = PathBuf::from(home);
            p.push(".local");
            p.push("state");
            p.push("mandala");
            p.push("history");
            return Some(p);
        }
    }
    None
}
