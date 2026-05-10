// SPDX-License-Identifier: MPL-2.0

//! `InputContext` — the active input focus that determines which
//! `Action` variants are eligible for a given key event. The event
//! loop derives the context from which modal is open; the resolver
//! filters through it.

/// The input context determines which `Action` variants are
/// eligible for a given key event. Each context has a parent;
/// if the context allows fallthrough, unmatched keys try the
/// parent. The root is `Document`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum_macros::IntoStaticStr)]
pub enum InputContext {
    /// No modal open. All global actions are eligible.
    Document,
    /// Console is open. Console-specific actions are tried first;
    /// unmatched keys do NOT fall through (console steals all input).
    Console,
    /// Color picker is open. Picker-specific actions are tried
    /// first; unmatched keys fall through to Document.
    ColorPicker,
    /// Label editor is open. Label-specific actions are tried
    /// first; unmatched keys do NOT fall through.
    LabelEdit,
    /// Text editor is open. Text-specific actions are tried
    /// first; unmatched keys do NOT fall through.
    TextEdit,
    /// `InteractionMode::NodeEdit { .. }` is active and the text
    /// editor is **not** open (entering the editor flips context to
    /// `TextEdit`). NodeEdit-specific Actions (`EnterSectionEdit`)
    /// are eligible here. Unmatched keys fall through to `Document`
    /// so global keybinds (Ctrl+S, Ctrl+Z, Esc → ExitMode) keep
    /// working inside NodeEdit.
    NodeEdit,
}

impl InputContext {
    /// Whether unmatched keys in this context should try the
    /// `Document` root. The fallthrough target is always
    /// `Document` — there's no general parent chain, just a
    /// modal-or-root distinction.
    pub fn falls_through(&self) -> bool {
        match self {
            InputContext::Document => false,
            InputContext::Console => false,
            InputContext::ColorPicker => true,
            InputContext::LabelEdit => false,
            InputContext::TextEdit => false,
            InputContext::NodeEdit => true,
        }
    }
}
