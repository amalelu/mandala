// SPDX-License-Identifier: MPL-2.0

//! Structural invariants for `.mindmap.json`: tree shape, Dewey IDs,
//! edge references, palette references, named enums, text-run
//! bounds, zoom-bound ordering. Each check returns `Vec<Violation>`;
//! `verify()` runs them all.

mod edges;
mod enums;
mod ids;
mod palettes;
mod references;
mod sections;
mod text_runs;
mod tree;
mod zoom_bounds;

#[cfg(test)]
mod test_helpers;

use baumhard::mindmap::model::{MindMap, MindNode};

/// Severity of a [`Violation`]. Warnings are printed but do not make
/// `maptool verify` exit nonzero — they flag likely mistakes (e.g.
/// section channel collisions) where the resulting behavior is still
/// well-defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub category: &'static str,
    pub location: String,
    pub message: String,
    pub severity: Severity,
}

impl Violation {
    /// Violation at a node's id.
    pub fn node(category: &'static str, node: &MindNode, message: impl Into<String>) -> Self {
        Self {
            category,
            location: node.id.clone(),
            message: message.into(),
            severity: Severity::Error,
        }
    }

    /// Warning at a node's id.
    pub fn node_warn(category: &'static str, node: &MindNode, message: impl Into<String>) -> Self {
        Self {
            category,
            location: node.id.clone(),
            message: message.into(),
            severity: Severity::Warning,
        }
    }

    /// Violation at `edge[<idx>]`.
    pub fn edge(category: &'static str, edge_index: usize, message: impl Into<String>) -> Self {
        Self {
            category,
            location: format!("edge[{}]", edge_index),
            message: message.into(),
            severity: Severity::Error,
        }
    }

    /// Violation at an arbitrary location (palette names, etc).
    pub fn at(category: &'static str, location: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            category,
            location: location.into(),
            message: message.into(),
            severity: Severity::Error,
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} @ {}: {}", self.category, self.location, self.message)
    }
}

/// Run all invariant checks and return every violation found.
/// An empty Vec means the file is valid.
pub fn verify(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();
    out.extend(tree::check(map));
    out.extend(ids::check(map));
    out.extend(references::check(map));
    out.extend(edges::check(map));
    out.extend(palettes::check(map));
    out.extend(enums::check(map));
    out.extend(text_runs::check(map));
    out.extend(sections::check(map));
    out.extend(zoom_bounds::check(map));
    out
}

#[cfg(test)]
mod constructor_tests {
    use super::*;

    /// `Display` formats as `<category> @ <location>: <message>`.
    /// Pinned because any drift from this format would silently
    /// break downstream verify-output parsing.
    #[test]
    fn violation_display_format_is_stable() {
        let v = Violation::edge("references", 0, "from_id missing");
        assert_eq!(format!("{}", v), "references @ edge[0]: from_id missing");
    }
}
