//! Well-known context tags for [`super::CustomMutation::contexts`].
//!
//! Contexts describe *where* and *on what* a mutation is meant to be
//! used. Dotted namespaces group related tags (`map.node`, `map.tree`);
//! a mutation may carry several. The strings below are the stable
//! surface — add new well-known tags here before any consumer
//! references them.
//!
//! The format follows `format/enums.md`: named snake_case strings, and
//! unknown tags are accepted on round-trip but don't match any
//! well-known predicate. `maptool verify` is free to flag unknown
//! roots in a future session without rejecting the load.
//!
//! ## Authoring convention
//!
//! - `internal` — an implementation-detail mutation registered by the
//!   host application for its own use. Not surfaced in `mutation list`
//!   and refused by `mutation apply`.
//! - `map` — this mutation operates on a mindmap (anything under the
//!   `map.*` sub-namespace inherits this root for filtering).
//! - `map.node` — touches the content of a single node (text, style,
//!   color, regions).
//! - `map.tree` — touches the tree structure / layout descending from
//!   a node (positions, children arrangement).
//!
//! Plugins reserve the `plugin.<name>.<kind>` sub-namespace for
//! third-party tags; the host crate will not introduce top-level
//! plugins of its own without a separate design cycle.

/// Internal application use only. Not exposed to the user; invoked
/// programmatically by app code.
pub const INTERNAL: &str = "internal";

/// Mutates a mindmap. Root namespace for any `map.*` context.
pub const MAP: &str = "map";

/// Mutates the content of a single node (text, style, color, regions).
pub const MAP_NODE: &str = "map.node";

/// Mutates tree structure / layout descending from a node (positions,
/// children arrangement, reparenting).
pub const MAP_TREE: &str = "map.tree";
