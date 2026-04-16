//! `InputContext` — the active input focus that determines which
//! `Action` variants are eligible for a given key event. The event
//! loop derives the context from which modal is open; the resolver
//! filters through it.

/// The input context determines which `Action` variants are
/// eligible for a given key event. Each context has a parent;
/// if the context allows fallthrough, unmatched keys try the
/// parent. The root is `Document`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
}

impl InputContext {
    /// Whether unmatched keys in this context should try the parent.
    pub fn falls_through(&self) -> bool {
        match self {
            InputContext::Document => false,
            InputContext::Console => false,
            InputContext::ColorPicker => true,
            InputContext::LabelEdit => false,
            InputContext::TextEdit => false,
        }
    }

    /// The parent context for fallthrough resolution.
    pub fn parent(&self) -> InputContext {
        InputContext::Document
    }
}
