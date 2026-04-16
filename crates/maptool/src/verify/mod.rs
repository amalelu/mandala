//! Verify a `.mindmap.json` file against the structural invariants
//! the format expects. Each submodule checks one category of
//! invariant and returns `Vec<Violation>`. `verify` runs them all
//! and returns the concatenated list.

mod enums;
mod ids;
mod palettes;
mod references;
mod text_runs;
mod tree;

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
    out
}
