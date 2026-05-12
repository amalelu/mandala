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
    ReplaceDocument(MindMapDocument),
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
/// `MindMapDocument` mutation through [`Self::document`].
pub struct ConsoleEffects<'a> {
    pub document: &'a mut MindMapDocument,
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
            side_effect: None,
            close_console: false,
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
    /// Emit multiple output lines. Each line carries an optional
    /// pinned font family — `font list` sets one per line so the
    /// row shapes in its own face, while help text / mutate-list
    /// tables leave it `None` and shape in the console default.
    Lines(Vec<OutputLine>),
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

impl ConsoleLine {
    pub fn text(&self) -> &str {
        match self {
            ConsoleLine::Input(s) | ConsoleLine::Error(s) => s,
            ConsoleLine::Output { text, .. } => text,
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
