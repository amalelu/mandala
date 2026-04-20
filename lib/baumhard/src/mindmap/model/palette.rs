//! Palette definitions: named color palettes referenced by nodes'
//! `color_schema.palette` field. Each palette defines a list of
//! color groups indexed by tree depth.

use serde::{Deserialize, Serialize};

use super::node::ColorGroup;

/// A named color palette — a list of per-depth [`ColorGroup`]s that
/// themed nodes index into via
/// [`super::node::ColorSchema::level`]. `groups[0]` is the root-level
/// color set; deeper levels walk the vector. Plain data; no runtime
/// cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Palette {
    pub groups: Vec<ColorGroup>,
}
