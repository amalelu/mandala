//! Runtime-value interface the [`super::build::build`] function
//! consults while expanding an AST. Consumers implement only the
//! methods their AST actually exercises; the others default to
//! `unreachable!()` so misuse is loud at runtime.

use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::mutator::Mutation;

pub trait SectionContext {
    /// Resolve a runtime cell count for `Repeat { count: Runtime(_) }`.
    fn count(&self, _name: &str) -> usize {
        unreachable!("section context does not supply runtime counts")
    }
    /// Look up the per-cell area for a `Repeat` section at iteration
    /// `index`. Required for any AST that materializes
    /// `MutationSrc::AreaDelta` inside a `Repeat`. The returned
    /// `&GlyphArea`'s lifetime is tied to the context — consumers
    /// typically precompute a `Vec<GlyphArea>` per section at build
    /// time and return a borrow.
    fn area(&self, _section: &str, _index: usize) -> &GlyphArea {
        unreachable!("section context does not supply per-section areas")
    }
    /// Resolve a runtime single `Mutation` for `MutationSrc::Runtime`.
    fn mutation(&self, _label: &str) -> Mutation {
        unreachable!("section context does not supply runtime mutations")
    }
    /// Resolve a runtime `Vec<Mutation>` for `MutationListSrc::Runtime`.
    fn mutation_list(&self, _label: &str) -> Vec<Mutation> {
        unreachable!("section context does not supply runtime mutation lists")
    }
}
