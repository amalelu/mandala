//! `Action` — the abstract user-action vocabulary the event loop
//! dispatches on. New keyboard interactions go here, then add a
//! matching `KeybindConfig` field + default + binding-string list.

use super::context::InputContext;

/// High-level user actions that can be bound to keys. Add a new variant
/// here when a new keyboard interaction is introduced, extend
/// `KeybindConfig` with a matching field + default, and handle the variant
/// in the event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    // ── Document-level (global) ──────────────────────────────────
    /// Undo the last action on the document.
    Undo,
    /// Enter reparent mode for the currently selected nodes.
    EnterReparentMode,
    /// Enter connect mode for the currently selected node.
    EnterConnectMode,
    /// Delete the current selection (currently: selected edge).
    DeleteSelection,
    /// Cancel the current mode (reparent / connect).
    CancelMode,
    /// Create a new unattached (orphan) node at the cursor position.
    CreateOrphanNode,
    /// Detach every currently selected node from its parent.
    OrphanSelection,
    /// Open the inline text editor on the currently selected single node
    /// with the node's existing text, cursor at end.
    EditSelection,
    /// Same as `EditSelection` but opens the editor with an empty buffer.
    EditSelectionClean,
    /// Open (or toggle) the CLI console.
    OpenConsole,
    /// Save the currently-open mindmap document to its bound file path.
    SaveDocument,
    /// Copy the focused component's clipboard representation.
    Copy,
    /// Paste the system clipboard's text content into the focused component.
    Paste,
    /// Cut: copy then clear the focused component's clipboard representation.
    Cut,

    // ── Console ──────────────────────────────────────────────────
    /// Close the console (two-tier: dismiss popup first, then close).
    ConsoleClose,
    /// Submit the current console input line for execution.
    ConsoleSubmit,
    /// Cycle tab completions.
    ConsoleTabComplete,
    /// Walk history backward / navigate completion popup upward.
    ConsoleHistoryUp,
    /// Walk history forward / navigate completion popup downward.
    ConsoleHistoryDown,
    /// Move cursor one grapheme left.
    ConsoleCursorLeft,
    /// Move cursor one grapheme right.
    ConsoleCursorRight,
    /// Move cursor to start of input.
    ConsoleCursorHome,
    /// Move cursor to end of input.
    ConsoleCursorEnd,
    /// Delete grapheme before cursor.
    ConsoleDeleteBack,
    /// Delete grapheme after cursor.
    ConsoleDeleteForward,
    /// Insert a literal space (winit delivers Space as Named, not Character).
    ConsoleInsertSpace,
    /// Clear the current input line (shell Ctrl+C muscle-memory).
    ConsoleClearLine,
    /// Jump cursor to start of line (shell Ctrl+A).
    ConsoleJumpStart,
    /// Jump cursor to end of line (shell Ctrl+E).
    ConsoleJumpEnd,
    /// Kill from cursor to start of line (shell Ctrl+U).
    ConsoleKillToStart,
    /// Kill the word before cursor (shell Ctrl+W).
    ConsoleKillWord,

    // ── Color Picker ─────────────────────────────────────────────
    /// Cancel the color picker (contextual mode only; ignored in standalone).
    PickerCancel,
    /// Commit the current color (contextual: close; standalone: apply to selection).
    PickerCommit,
    /// Nudge hue −15°.
    PickerNudgeHueDown,
    /// Nudge hue +15°.
    PickerNudgeHueUp,
    /// Nudge saturation −0.1.
    PickerNudgeSatDown,
    /// Nudge saturation +0.1.
    PickerNudgeSatUp,
    /// Nudge value −0.1.
    PickerNudgeValDown,
    /// Nudge value +0.1.
    PickerNudgeValUp,

    // ── Label Editor ─────────────────────────────────────────────
    /// Cancel the inline label editor (discard changes).
    LabelEditCancel,
    /// Commit the inline label editor.
    LabelEditCommit,

    // ── Text Editor ──────────────────────────────────────────────
    /// Cancel the inline text editor (discard changes).
    TextEditCancel,
}

impl Action {
    /// The input context this action belongs to. Used by the
    /// contextual resolver to filter which actions are eligible
    /// in a given modal state.
    pub fn context(&self) -> InputContext {
        match self {
            Action::ConsoleClose
            | Action::ConsoleSubmit
            | Action::ConsoleTabComplete
            | Action::ConsoleHistoryUp
            | Action::ConsoleHistoryDown
            | Action::ConsoleCursorLeft
            | Action::ConsoleCursorRight
            | Action::ConsoleCursorHome
            | Action::ConsoleCursorEnd
            | Action::ConsoleDeleteBack
            | Action::ConsoleDeleteForward
            | Action::ConsoleInsertSpace
            | Action::ConsoleClearLine
            | Action::ConsoleJumpStart
            | Action::ConsoleJumpEnd
            | Action::ConsoleKillToStart
            | Action::ConsoleKillWord => InputContext::Console,

            Action::PickerCancel
            | Action::PickerCommit
            | Action::PickerNudgeHueDown
            | Action::PickerNudgeHueUp
            | Action::PickerNudgeSatDown
            | Action::PickerNudgeSatUp
            | Action::PickerNudgeValDown
            | Action::PickerNudgeValUp => InputContext::ColorPicker,

            Action::LabelEditCancel
            | Action::LabelEditCommit => InputContext::LabelEdit,

            Action::TextEditCancel => InputContext::TextEdit,

            _ => InputContext::Document,
        }
    }
}
