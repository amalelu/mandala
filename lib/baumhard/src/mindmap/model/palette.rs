//! Palette definitions: named color palettes referenced by nodes'
//! `color_schema.palette` field. Each palette defines a list of
//! color groups indexed by tree depth.

use serde::{Deserialize, Serialize};

use super::node::ColorGroup;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Palette {
    pub groups: Vec<ColorGroup>,
}
