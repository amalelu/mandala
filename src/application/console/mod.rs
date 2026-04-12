//! CLI-style console for Mandala — the successor to the Session 6C
//! command palette.
//!
//! The console is the lowest-level, most-powerful keyboard surface:
//! every former palette action is a command, plus new verbs that the
//! palette couldn't express (help, selection navigation, mutate
//! list/run/bind, aliases, history).
//!
//! Input is tokenized shell-style: whitespace splits, `"quoted
//! strings"` preserve spaces, `--flag=value` / `--flag value` parse.
//! Completion is contextual — tab expands a partial command name at
//! position 0, enum args inside known commands, or known ids.
//!
//! The command registry is `const COMMANDS: &[Command]`, matching the
//! zero-cost-startup property the palette had.
//!
//! Module layout:
//!
//! - [`fuzzy`] — the subsequence scoring algorithm (verbatim move).
//! - [`parser`] — `tokenize`, `parse`, `Args`.
//! - [`predicates`] — applicability helpers (selection-shape queries).
//! - [`commands`] — the `COMMANDS` slice and per-command exec logic.
//! - [`completion`] — tab-completion engine.

use crate::application::color_picker::ColorTarget;
use crate::application::document::{EdgeRef, MindMapDocument};
use baumhard::mindmap::custom_mutation::CustomMutation;
use std::collections::HashMap;

pub mod commands;
pub mod completion;
pub mod constants;
pub mod fuzzy;
pub mod parser;
pub mod predicates;
pub mod user_mutations;

#[cfg(test)]
mod tests;

// Re-exports kept narrow — only what crosses module boundaries is
// surfaced. The rest stays reachable via the submodule path for
// grep-ability.
#[allow(unused_imports)]
pub use fuzzy::fuzzy_score;
#[allow(unused_imports)]
pub use parser::{parse, tokenize, Args, ParseResult};

/// Read-only view of app state for applicability checks, completion,
/// and informational commands (`help`, `mutate list`).
pub struct ConsoleContext<'a> {
    pub document: &'a MindMapDocument,
    /// The merged mutation registry (user + map + inline). Passed in
    /// as a reference rather than re-deriving from the document so
    /// tests can construct synthetic registries without a full map.
    /// In normal use this is `&document.mutation_registry`.
    pub mutation_registry: &'a HashMap<String, CustomMutation>,
}

impl<'a> ConsoleContext<'a> {
    /// Convenience constructor that pulls the registry straight off
    /// the document — the shape the app event loop uses.
    pub fn from_document(document: &'a MindMapDocument) -> Self {
        Self {
            document,
            mutation_registry: &document.mutation_registry,
        }
    }
}

/// Mutable handles handed to `execute`. Superset of the former
/// `PaletteEffects` — keeps the two modal-handoff fields the palette
/// already used, plus `run_mutation` for `mutate run` (needs tree
/// access that only the event-loop dispatcher has).
pub struct ConsoleEffects<'a> {
    pub document: &'a mut MindMapDocument,
    /// If set when `execute` returns, the dispatcher transitions to
    /// the inline label editor on the given edge.
    pub open_label_edit: Option<EdgeRef>,
    /// If set when `execute` returns, the dispatcher transitions to
    /// the glyph-wheel color picker on the given target.
    pub open_color_picker: Option<ColorTarget>,
    /// If set when `execute` returns, the dispatcher closes the
    /// console even on a successful command (e.g. `quit`, or after a
    /// modal handoff).
    pub close_console: bool,
    /// If set when `execute` returns, the dispatcher calls
    /// `MindMapDocument::apply_custom_mutation` with these
    /// arguments. Exposed as a deferred request because
    /// `apply_custom_mutation` needs `&mut MindMapTree`, which the
    /// command fn doesn't have access to (it only holds the
    /// document).
    pub run_mutation: Option<RunMutationRequest>,
}

#[derive(Clone, Debug)]
pub struct RunMutationRequest {
    pub mutation_id: String,
    pub node_id: String,
}

impl<'a> ConsoleEffects<'a> {
    pub fn new(document: &'a mut MindMapDocument) -> Self {
        Self {
            document,
            open_label_edit: None,
            open_color_picker: None,
            close_console: false,
            run_mutation: None,
        }
    }
}

/// Outcome of a single `execute` call. All variants eventually
/// manifest as a line in the console scrollback; `Err` and `Ok`
/// differ only in the color they render.
#[derive(Debug)]
pub enum ExecResult {
    /// Success with an optional message to append to the scrollback.
    /// Commands that didn't produce notable output return
    /// `Ok(String::new())` — the dispatcher suppresses empty Ok
    /// lines.
    Ok(String),
    /// Failed execution with a diagnostic message.
    Err(String),
    /// Emit multiple lines of output (help text, `mutate list`
    /// tables, etc.).
    Lines(Vec<String>),
}

impl ExecResult {
    pub fn ok_empty() -> Self {
        ExecResult::Ok(String::new())
    }
    pub fn ok_msg(s: impl Into<String>) -> Self {
        ExecResult::Ok(s.into())
    }
    pub fn err(s: impl Into<String>) -> Self {
        ExecResult::Err(s.into())
    }
}

/// One rendered line in the scrollback. Colored at render time by
/// variant.
#[derive(Clone, Debug)]
pub enum ConsoleLine {
    /// Echo of a user-entered command (`> anchor set from auto`).
    Input(String),
    /// Normal output line from a command.
    Output(String),
    /// Error output from a failed command.
    Error(String),
}

impl ConsoleLine {
    pub fn text(&self) -> &str {
        match self {
            ConsoleLine::Input(s) | ConsoleLine::Output(s) | ConsoleLine::Error(s) => s,
        }
    }
}

/// Console UI state. Mirrors the `PaletteState` shape — either
/// `Closed` or `Open { ... }`, with the whole line-editor +
/// scrollback living in the `Open` arm.
#[derive(Clone, Debug)]
pub enum ConsoleState {
    Closed,
    Open {
        /// Current input buffer. Not shell-expanded; that happens at
        /// `parse` time on Enter.
        input: String,
        /// Grapheme-cluster index into `input` where the cursor
        /// sits. Edits go through `baumhard::util::grapheme_chad`
        /// helpers (`insert_str_at_grapheme`, `delete_grapheme_at`,
        /// `count_grapheme_clusters`, `find_byte_index_of_grapheme`)
        /// so ZWJ emoji / flag sequences / combining marks are
        /// treated as single cursor cells — per CODE_CONVENTIONS §2.
        cursor: usize,
        /// Past commands, oldest first. Up/Down scrolls an index into
        /// this vec; appended on every `Enter`.
        history: Vec<String>,
        /// `None` while editing a fresh line; `Some(idx)` after the
        /// user pressed Up — then subsequent Up/Down walks the
        /// history, restoring to a fresh empty line when we scroll
        /// past the newest entry.
        history_idx: Option<usize>,
        /// Rendered scrollback (echoed commands + output). The
        /// renderer shows the trailing N lines.
        scrollback: Vec<ConsoleLine>,
        /// Computed completion candidates. Populated lazily on Tab;
        /// cleared on every input change so a stale popup doesn't
        /// shadow the new context.
        completions: Vec<completion::Completion>,
        /// Which completion is highlighted. `None` when the popup is
        /// closed (no completions computed yet); `Some(idx)` after Tab.
        completion_idx: Option<usize>,
    },
}

impl ConsoleState {
    pub fn is_open(&self) -> bool {
        matches!(self, ConsoleState::Open { .. })
    }

    /// Construct a fresh open state seeded with the given history.
    /// The dispatcher in `app.rs` owns `history` across sessions and
    /// passes the persisted list in here on every open.
    pub fn open(history: Vec<String>) -> Self {
        ConsoleState::Open {
            input: String::new(),
            cursor: 0,
            history,
            history_idx: None,
            scrollback: Vec::new(),
            completions: Vec::new(),
            completion_idx: None,
        }
    }
}

/// Hard cap for persisted history length. The file is rotated when
/// the on-disk size exceeds `2 * MAX_HISTORY`.
pub const MAX_HISTORY: usize = 500;
