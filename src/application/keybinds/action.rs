//! `Action` — the abstract user-action vocabulary the event loop
//! dispatches on. New keyboard interactions go here, then add a
//! matching `KeybindConfig` field + default + binding-string list.

/// High-level user actions that can be bound to keys. Add a new variant
/// here when a new keyboard interaction is introduced, extend
/// `KeybindConfig` with a matching field + default, and handle the variant
/// in the event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
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
    /// Create a new unattached (orphan) node at the cursor position. The
    /// node starts with no parent so users can build a piece in isolation
    /// and attach it with reparent mode (Ctrl+P) later.
    CreateOrphanNode,
    /// Detach every currently selected node from its parent, promoting it
    /// to a root node. Each selected node's full subtree stays attached to
    /// it — this only severs the link between the selection and its
    /// former parent, not the selection and its children.
    OrphanSelection,
    /// Open the inline text editor on the currently selected single node
    /// with the node's existing text, cursor at end. Paired with
    /// `EditSelectionClean` which opens with an empty buffer instead.
    /// Only fires at the document level — the text-edit steal at the
    /// top of keyboard dispatch means this action can't collide with
    /// editor-mode Enter/Backspace.
    EditSelection,
    /// Same as `EditSelection` but opens the editor with an empty buffer.
    /// On commit the node's text is replaced wholesale — the "clean
    /// slate" gesture: press Backspace on a selected node to retype it
    /// from scratch.
    EditSelectionClean,
    /// Open (or toggle) the CLI console. Suppressed while any other
    /// keyboard-capturing modal is active (`LabelEditState`,
    /// `ColorPickerState`, `TextEditState`). Pressing again while the
    /// console is open closes it — symmetric with Esc and the shell
    /// muscle memory around a toggle-open console.
    OpenConsole,
    /// Save the currently-open mindmap document to its bound file
    /// path. If no `file_path` is set (e.g. after `new` without a
    /// path), the action is a no-op aside from a status message —
    /// the user has to invoke `save <path>` from the console first
    /// to bind a target. WASM builds have no filesystem access, so
    /// the action is logged and ignored there.
    SaveDocument,
}
