//! Capability traits and the [`TargetView`] dispatcher — the core of
//! the console's trait-dispatched cross-cutting command layer.
//!
//! The idea: a command like `color bg=accent text=#fff` doesn't know
//! whether the current selection is a node, an edge, or a portal. It
//! materializes a `Vec<TargetView>` from `SelectionState` (a single
//! node, or five multi-selected nodes, or one edge, or one portal)
//! and for each kv-pair calls the corresponding trait method on every
//! target. The target variant that doesn't implement the trait
//! returns [`Outcome::NotApplicable`]; the dispatcher aggregates the
//! outcomes into a single per-kv report.
//!
//! Why `enum TargetView` and not `Box<dyn Trait>`: the set of targets
//! is closed (Node / Edge / Portal) and small, `match` is trivially
//! cheap, and avoiding dynamic dispatch keeps the signatures shorter
//! (no `dyn HasBgColor`). The same principle is in use across the
//! baumhard crate for the mindmap model.
//!
//! Module split:
//! - [`color_value`] — the `ColorValue` parser.
//! - [`outcome`] — the per-trait-call `Outcome` result type.
//! - [`capabilities`] — the six capability trait definitions.
//! - [`view`] — `TargetView` enum + per-trait impls + materialization.
//! - [`dispatch`] — `apply_kvs` + `DispatchReport`.

mod capabilities;
mod color_value;
mod dispatch;
mod outcome;
mod view;

#[cfg(test)]
mod tests;

pub use capabilities::{
    AcceptsWheelColor, HasBgColor, HasBorderColor, HasFontSize, HasLabel, HasTextColor,
};
pub use color_value::ColorValue;
pub use dispatch::{apply_kvs, DispatchReport};
pub use outcome::Outcome;
pub use view::{selection_targets, view_for, TargetId, TargetView};
