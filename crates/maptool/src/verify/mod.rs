//! Structural invariant verification for `.mindmap.json` files.
//!
//! Verification is a boundary check, not a best-effort parse: a file
//! either satisfies every named invariant the format guarantees or it
//! doesn't, and each violation is reported as a specific, named
//! property (tree shape, Dewey-ID consistency, edge references, palette
//! references, named-enum membership, text-run bounds) rather than a
//! free-form error message. That separation is what makes `verify` safe
//! to run as a gate: the loader can be permissive about missing or
//! defaulted fields, and everything load-tolerant-but-structurally-
//! invalid still surfaces here.

mod enums;
mod ids;
mod palettes;
mod references;
mod text_runs;
mod tree;
mod zoom_bounds;

#[cfg(test)]
mod test_helpers;

use baumhard::mindmap::model::MindMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub category: &'static str,
    pub location: String,
    pub message: String,
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
    out.extend(palettes::check(map));
    out.extend(enums::check(map));
    out.extend(text_runs::check(map));
    out.extend(zoom_bounds::check(map));
    out
}
