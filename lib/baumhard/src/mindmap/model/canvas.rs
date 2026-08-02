// SPDX-License-Identifier: MPL-2.0

//! Canvas — the per-map rendering context: background color, default
//! border / connection styles applied when no per-node or per-edge
//! override exists, and the live theme-variable map that `var(--name)`
//! color references resolve against.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{GlyphBorderConfig, GlyphConnectionConfig};

/// Shared, per-map rendering context: background color, default
/// border / connection styles, live theme-variable map, and the
/// named theme variants that swap into it. One `Canvas` per
/// [`super::MindMap`]. Plain data; no runtime cost beyond the
/// `HashMap` / `String` allocations serde performs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Canvas {
    pub background_color: String,
    /// Default border style applied to all nodes unless overridden per-node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_border: Option<GlyphBorderConfig>,
    /// Default connection style applied to all edges unless overridden per-edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_connection: Option<GlyphConnectionConfig>,
    /// Default section-frame style for sections of nodes in
    /// NodeEdit mode. `None` → hardcoded thin floor. Set on the
    /// canvas to give an entire map a coherent section-frame look
    /// without writing per-section overrides on every node. Authors
    /// reach this via the `canvas section-frame …` console subverb.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_section_frame_border: Option<GlyphBorderConfig>,
    /// Default section-frame style for the *focused* section (the
    /// one currently inside the open inline text editor). `None` →
    /// hardcoded heavy floor. Same cascade as
    /// [`Self::default_section_frame_border`]; the focused variant
    /// gives the focused section a visually distinct frame so the
    /// user sees which section is being edited among siblings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_focused_section_frame_border: Option<GlyphBorderConfig>,
    /// The live map of theme variables, each keyed by its CSS-style name
    /// (including the leading `--`, e.g. `"--bg"`). Any color string in the
    /// map can reference these via `var(--name)` and will be resolved at
    /// scene-build time. This is the single source of truth for the "current
    /// theme"; switching themes copies a preset from `theme_variants` into
    /// this map.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub theme_variables: HashMap<String, String>,
    /// Named theme presets. Values are whole variable maps that can be
    /// copied into `theme_variables` via a `SetThemeVariant` document
    /// action. Editing a variant here does nothing at runtime until it's
    /// activated — these are authoring state, not the live theme.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub theme_variants: HashMap<String, HashMap<String, String>>,
}
