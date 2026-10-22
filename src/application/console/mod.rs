// SPDX-License-Identifier: MPL-2.0

//! CLI-style console for Mandala.
//!
//! Input is tokenized shell-style — whitespace splits, `"quoted
//! strings"` preserve spaces, and `key=value` tokens are first-class.
//! Generic commands (`color` / `font` / `label`) dispatch through
//! the [`traits`] capability traits and fan out over the current
//! selection via [`traits::TargetView`]; component-specific commands
//! (`anchor` / `body` / `cap` / `spacing` / `edge` / `portal`) call
//! their own `MindMapDocument` setters directly.
//!
//! Completion is contextual and prefix-matched. The popup recomputes
//! on every keystroke; `↑`/`↓` move the highlight (falling back to
//! command history when the popup is empty), `Tab` accepts the
//! highlighted row, `Esc` dismisses the popup, then the console.

use crate::application::color_picker::ColorTarget;
use crate::application::common::FpsDisplayMode;
use crate::application::document::{EdgeRef, MindMapDocument};

pub mod commands;
pub mod completion;
pub mod constants;
pub mod helpers;
pub mod parser;
pub mod predicates;
pub mod spec;
pub mod traits;
pub mod visuals;

#[cfg(test)]
pub(in crate::application::console) mod tests;

// Re-exports kept narrow — only what crosses module boundaries is
// surfaced. The rest stays reachable via the submodule path for
// grep-ability.
#[allow(unused_imports)]
pub use parser::{parse, tokenize, Args, ParseResult};

/// Read-only view of app state for applicability checks, completion,
/// and informational commands (e.g. `help`).
pub struct ConsoleContext<'a> {
    pub document: &'a MindMapDocument,
}

impl<'a> ConsoleContext<'a> {
    /// Convenience constructor — the shape the app event loop uses.
    pub fn from_document(document: &'a MindMapDocument) -> Self {
        Self { document }
    }
}

/// One out-of-band effect a console command can request the
/// dispatcher to perform after the command's `MindMapDocument`
/// mutations land. All variants are mutually exclusive — a
/// single command produces at most one transition / state write.
/// `close_console` lives outside the enum because it's orthogonal
/// (a transition can leave the console open or close it).
pub enum ConsoleSideEffect {
    /// Transition to the inline label editor on the given edge.
    OpenLabelEdit(EdgeRef),
    /// Transition to the inline portal-text editor on the given
    /// `(edge_ref, endpoint_node_id)` pair. The dispatcher picks
    /// this vs [`Self::OpenLabelEdit`] based on the source
    /// command's selection variant.
    OpenPortalTextEdit(EdgeRef, String),
    /// Transition to the glyph-wheel color picker in
    /// **contextual** mode on the given target. Commit writes to
    /// that target and closes; Esc / outside-click cancel.
    OpenColorPicker(ColorTarget),
    /// Transition to the glyph-wheel color picker in
    /// **standalone** mode — a persistent palette with no bound
    /// target. Commit applies the current HSV to the document's
    /// current selection; the palette stays open until
    /// [`Self::CloseColorPicker`] is requested. Set by
    /// `color picker on`.
    OpenColorPickerStandalone,
    /// Close any open color picker (contextual or standalone)
    /// without committing. Set by `color picker off`.
    CloseColorPicker,
    /// Forward `mode` to `Renderer::set_fps_display`. Set by
    /// `fps on` (Snapshot), `fps debug` (Debug), `fps off` (Off).
    SetFpsDisplay(FpsDisplayMode),
    /// Wholesale document swap. Set by `open` and `new`. The
    /// dispatcher also drops the cached `mindmap_tree` and clears
    /// any open modal-editor state so stale references into the
    /// old document can't outlive the swap.
    ///
    /// **Boxed.** A whole document is by far the largest payload here,
    /// and every effect — a one-field `OpenLabelEdit` included — is
    /// returned by value, so the enum would otherwise be a document
    /// wide for all of them.
    ReplaceDocument(Box<MindMapDocument>),
    /// Flip the high-level interaction mode and trigger a scene
    /// rebuild. Mirrors the `SetFpsDisplay` precedent: a UI-state
    /// transition that doesn't pass through `Action` because the
    /// console verb already IS the user-named effect (no second
    /// dispatch site needed). Used by the `mode` console verb's
    /// `default` / `resize` subverbs. `default` carries
    /// `InteractionMode::Default`; `resize` carries
    /// `InteractionMode::Resize { ... }` resolved from the
    /// selection inside the verb body.
    SetInteractionMode(crate::application::app::InteractionMode),
    /// Enter the section text editor on `(node_id, section_idx)`.
    /// Set by the `section edit [section=<idx>]` console verb.
    /// The dispatcher splits the work: pre-rebuild writes
    /// `SelectionState::Section { node_id, section_idx }` +
    /// `InteractionMode::NodeEdit { node_id }`; post-rebuild
    /// delegates to `apply_enter_section_edit` (the canonical
    /// Action-side path used by `Action::EnterSectionEdit` on
    /// the keybind path), which validates `OwnerMismatch` and
    /// opens the editor. Single source of truth for "open
    /// section editor"; the verb is the only producer of this
    /// variant.
    OpenSectionEdit { node_id: String, section_idx: usize },
}

impl std::fmt::Debug for ConsoleSideEffect {
    /// Test-friendly variant tag without recursing into the
    /// (non-Debug) `MindMapDocument` payload. Tests use
    /// `matches!(eff.side_effect, ConsoleSideEffect::X(_))` so
    /// the variant discrimination is what matters.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenLabelEdit(er) => f.debug_tuple("OpenLabelEdit").field(er).finish(),
            Self::OpenPortalTextEdit(er, n) => {
                f.debug_tuple("OpenPortalTextEdit").field(er).field(n).finish()
            }
            Self::OpenColorPicker(t) => f.debug_tuple("OpenColorPicker").field(t).finish(),
            Self::OpenColorPickerStandalone => write!(f, "OpenColorPickerStandalone"),
            Self::CloseColorPicker => write!(f, "CloseColorPicker"),
            Self::SetFpsDisplay(m) => f.debug_tuple("SetFpsDisplay").field(m).finish(),
            Self::ReplaceDocument(_) => write!(f, "ReplaceDocument(<doc>)"),
            Self::SetInteractionMode(m) => f.debug_tuple("SetInteractionMode").field(m).finish(),
            Self::OpenSectionEdit { node_id, section_idx } => f
                .debug_struct("OpenSectionEdit")
                .field("node_id", node_id)
                .field("section_idx", section_idx)
                .finish(),
        }
    }
}

/// Mutable handles handed to `execute`. The dispatcher reads
/// [`Self::side_effect`] and [`Self::close_console`] after the
/// command returns; everything else is a direct
/// `MindMapDocument` mutation through [`Self::document_mut`].
///
/// **Reading and writing the document are different calls on
/// purpose.** The document is reached through
/// [`Self::document`] (shared) or [`Self::document_mut`]
/// (exclusive), and taking the exclusive one raises a flag the
/// dispatcher reads back through [`Self::document_mutated`]. That
/// flag is what gates the post-command `scene_cache.clear()` +
/// `rebuild_all`: `help`, `fps` and `mutation list` never take a
/// `&mut`, so they no longer pay for a rebuild of a document they
/// only read.
///
/// The signal is deliberately **the borrow, not the write**.
/// Marking on `&mut` over-reports — a `border reset` that turns out
/// to be a no-op still counts — and that is the safe direction: the
/// cost of over-reporting is one rebuild the user would have paid
/// for anyway, while the cost of under-reporting is a canvas that
/// silently disagrees with the model. It is also the only version a
/// new command cannot forget, because there is no way to write to
/// the document without going through the call that raises it.
pub struct ConsoleEffects<'a> {
    document: &'a mut MindMapDocument,
    /// Set by [`Self::document_mut`]. See the type's doc comment for
    /// why the borrow rather than the write is the signal.
    document_mutated: bool,
    /// Out-of-band transition / state write the dispatcher should
    /// perform after the command. Mutually exclusive — at most
    /// one effect per command.
    pub side_effect: Option<ConsoleSideEffect>,
    /// Close the console after the command, even on success
    /// (e.g. `quit`, or after a modal handoff that takes the
    /// user out of the console). Orthogonal to `side_effect`.
    pub close_console: bool,
}

impl<'a> ConsoleEffects<'a> {
    pub fn new(document: &'a mut MindMapDocument) -> Self {
        Self {
            document,
            document_mutated: false,
            side_effect: None,
            close_console: false,
        }
    }

    /// Read the document. Leaves [`Self::document_mutated`] alone.
    pub fn document(&self) -> &MindMapDocument {
        self.document
    }

    /// Take the document exclusively, marking the command as one
    /// that may have changed what is drawn.
    pub fn document_mut(&mut self) -> &mut MindMapDocument {
        self.document_mutated = true;
        self.document
    }

    /// Whether the command took [`Self::document_mut`].
    pub fn document_mutated(&self) -> bool {
        self.document_mutated
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
    /// Emit multiple output lines. Each line carries an optional
    /// pinned font family — `font list` sets one per line so the
    /// row shapes in its own face, while help text / mutate-list
    /// tables leave it `None` and shape in the console default.
    Lines(Vec<OutputLine>),
}

/// The diagnostic code a failed console line is logged under when a
/// caller other than the prompt ran it (a file Action, a macro step,
/// an IPC command): `log::warn!(code = LINE_FAILED_CODE; …)`. An IPC
/// reply carries it as a `console_error` warning, matching the
/// `console_error` error code `act.console` answers with, so an agent
/// can branch on a console failure however the line was reached. A code
/// is API there (`format/ipc.md` §Warnings).
pub const LINE_FAILED_CODE: &str = "console_error";

/// The diagnostic code `Action::SaveDocument` logs when no file path
/// is bound, so there is nothing to save to. A save-as fixes it. An IPC
/// reply carries it as a `save_unbound` warning (`format/ipc.md`
/// §Warnings).
pub const SAVE_UNBOUND_CODE: &str = "save_unbound";

/// The diagnostic code `Action::SaveDocument` logs when writing the
/// bound file failed: the disk, the path, permissions. An IPC reply
/// carries it as a `save_failed` warning.
pub const SAVE_FAILED_CODE: &str = "save_failed";

/// What running one console line produced, before anything renders
/// it. The console modal folds it into its scrollback. The IPC
/// `act.console` command returns it to the client. Neither path
/// changes what the verb did.
#[derive(Debug)]
#[must_use = "a console line's outcome is its only report: print it, log it, or return it"]
pub enum LineOutcome {
    /// The line was blank, so nothing ran.
    Blank,
    /// The line's head names no verb. Carries the head token.
    Unknown(String),
    /// A verb ran and returned this.
    Ran(ExecResult),
}

impl LineOutcome {
    /// Whether the line did what it asked: a verb ran and did not
    /// report a failure. A blank line, an unknown verb and a verb
    /// error did not apply.
    pub fn applied(&self) -> bool {
        matches!(self, LineOutcome::Ran(ExecResult::Ok(_) | ExecResult::Lines(_)))
    }

    /// The failure text the console shows for this outcome, or
    /// `None` when the line succeeded or was blank. One spelling,
    /// shared by the scrollback, the log and the IPC reply.
    pub fn failure(&self) -> Option<String> {
        match self {
            LineOutcome::Unknown(head) => Some(format!("unknown command: {head}")),
            LineOutcome::Ran(ExecResult::Err(message)) => Some(message.clone()),
            LineOutcome::Blank | LineOutcome::Ran(ExecResult::Ok(_) | ExecResult::Lines(_)) => None,
        }
    }
}

/// One line of multi-line console output. Default-shaped (font
/// family unset) unless the producing command pins a face — used
/// today by `font list` so each row renders in the family it
/// names.
#[derive(Clone, Debug, Default)]
pub struct OutputLine {
    pub text: String,
    pub font_family: Option<String>,
}

impl OutputLine {
    /// Plain text, console default font.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            font_family: None,
        }
    }

    /// Text shaped in `family`.
    pub fn in_font(text: impl Into<String>, family: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            font_family: Some(family.into()),
        }
    }
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
    /// Convenience for command handlers that emit plain
    /// console-default-font output. Mirrors the pre-collapse
    /// `Lines(Vec<String>)` ergonomics so existing call sites
    /// stay one-liners.
    pub fn lines<I, S>(lines: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        ExecResult::Lines(lines.into_iter().map(OutputLine::plain).collect())
    }
}

/// One rendered line in the scrollback. Colored at render time by
/// variant. The `Output` variant additionally carries an optional
/// `font_family` — set by `font list` so each font name shapes in
/// its own face. `Input` echoes and `Error` lines always use the
/// console default font.
#[derive(Clone, Debug)]
pub enum ConsoleLine {
    /// Echo of a user-entered command (`> anchor set from auto`).
    Input(String),
    /// Normal output line from a command.
    Output {
        text: String,
        /// Pinned font family for this line, or `None` for the
        /// console default. Set by commands like `font list` that
        /// want each row in its own face.
        font_family: Option<String>,
    },
    /// Error output from a failed command.
    Error(String),
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
        /// Scrollback view offset. `0` means "pinned to the bottom"
        /// (the trailing N lines fill the visible window). `N` means
        /// "the visible window's bottom edge sits N lines above the
        /// newest line" — i.e. the user has scrolled up by N. Clamped
        /// at read time against
        /// `scrollback.len().saturating_sub(MAX_CONSOLE_SCROLLBACK_ROWS)`
        /// so growing scrollback can never strand the offset
        /// out-of-range. Reset to `0` on any input change or new
        /// scrollback arrival so the next command shows in view.
        scroll_offset: usize,
        /// Mousewheel-line accumulator. Wheel deltas arrive as fixed
        /// pixel amounts (or per-platform line counts) that are
        /// rarely a clean multiple of one line. We accumulate the
        /// fractional remainder here so a slow scroll with
        /// sub-line-per-tick deltas still moves at all.
        wheel_accum: f32,
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
            scroll_offset: 0,
            wheel_accum: 0.0,
        }
    }
}

/// Hard cap for persisted history length. The file is rotated when
/// the on-disk size exceeds `2 * MAX_HISTORY`.
pub const MAX_HISTORY: usize = 500;
