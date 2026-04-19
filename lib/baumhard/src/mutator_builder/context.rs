//! Runtime-value interface the [`super::build::build`] function
//! consults while expanding an AST. Consumers implement only the
//! methods their AST actually exercises; the others default to
//! `unreachable!()` so misuse is loud at runtime.

use crate::gfx_structs::area::{GlyphArea, GlyphAreaField};
use crate::gfx_structs::mutator::Mutation;

use super::ast::CellField;

/// Runtime look-up surface for the mutator builder. Each method
/// corresponds to one runtime-sourced AST variant; consumers implement
/// only those their `MutatorNode` exercises.
///
/// The default implementations panic loudly via `unreachable!()` so a
/// misused AST fails immediately rather than silently producing wrong
/// mutator trees.
pub trait SectionContext {
    /// Resolve a runtime cell count for `Repeat { count: Runtime(_) }`.
    /// `name` is the `Runtime` label the AST carries; the consumer
    /// disambiguates among multiple `Repeat` sections by this label.
    /// Called once per `Repeat` node during [`super::build::build`].
    ///
    /// Cost: consumer-defined, typically O(1) via a small HashMap.
    fn count(&self, _name: &str) -> usize {
        unreachable!("section context does not supply runtime counts")
    }
    /// Look up the per-cell area for a `Repeat` section at iteration
    /// `index`. Required for any AST that materializes
    /// `MutationSrc::AreaDelta` inside a `Repeat` *unless* the
    /// consumer overrides [`field`](Self::field) — see that method's
    /// docs for the slim-context opt-out.
    ///
    /// Cost: consumer-defined. Typically a slice index.
    fn area(&self, _section: &str, _index: usize) -> &GlyphArea {
        unreachable!("section context does not supply per-section areas")
    }
    /// Materialize one `GlyphAreaField` for a `Repeat` section at
    /// iteration `index`. The default implementation calls
    /// [`area`](Self::area) and projects the requested field out of
    /// the returned `GlyphArea` — the historical shape.
    ///
    /// Override this on per-frame hot paths (the picker's dynamic
    /// phase is the canonical example) when the dynamic spec touches
    /// only a handful of fields per cell, so the context can build
    /// *only* those field values without allocating a full
    /// `GlyphArea` per cell per frame. Implementations that override
    /// `field` may leave `area` at its default `unreachable!()`.
    fn field(&self, section: &str, index: usize, template: &CellField) -> GlyphAreaField {
        default_field_from_area(self.area(section, index), template)
    }
    /// Resolve a runtime single `Mutation` for `MutationSrc::Runtime`.
    /// `label` is the enclosing `Repeat` section's name (or `""` when
    /// invoked outside a `Repeat` template). Called once per
    /// materialised `Single`/`Instruction` carrying a `Runtime`
    /// mutation. Returning a `Mutation::None` is a valid no-op choice.
    ///
    /// Cost: consumer-defined. The caller holds no locks; the
    /// implementation owns the mutation it returns (cloning is
    /// implicit in the return-by-value contract).
    fn mutation(&self, _label: &str) -> Mutation {
        unreachable!("section context does not supply runtime mutations")
    }
    /// Resolve a runtime `Vec<Mutation>` for `MutationListSrc::Runtime`.
    /// `label` is the free-form key the AST carries; the consumer
    /// disambiguates among multiple runtime-sourced macros by this
    /// label. Called once per matching `Macro` node during
    /// [`super::build::build`]. Returning an empty `Vec` is a valid
    /// no-op choice; the built `GfxMutator::Macro` then walks over
    /// its target without applying anything.
    ///
    /// Cost: consumer-defined. The returned `Vec` is moved into the
    /// mutator tree, so no double-allocation.
    fn mutation_list(&self, _label: &str) -> Vec<Mutation> {
        unreachable!("section context does not supply runtime mutation lists")
    }
}

/// Default projection used by `SectionContext::field`: pulls each
/// field out of a pre-built `GlyphArea`. Only consumed by the
/// trait's own default implementation today — kept `pub(super)`
/// because no consumer outside `mutator_builder` needs to reach
/// for it; slim overrides that want a subset projection can call
/// it via the trait default by delegating to `area`.
pub(super) fn default_field_from_area(area: &GlyphArea, template: &CellField) -> GlyphAreaField {
    match template {
        CellField::Text => GlyphAreaField::Text(area.text.clone()),
        CellField::position => GlyphAreaField::position(area.position.x.0, area.position.y.0),
        CellField::bounds => {
            GlyphAreaField::bounds(area.render_bounds.x.0, area.render_bounds.y.0)
        }
        CellField::scale => GlyphAreaField::scale(area.scale.0),
        CellField::line_height => GlyphAreaField::line_height(area.line_height.0),
        CellField::ColorFontRegions => GlyphAreaField::ColorFontRegions(area.regions.clone()),
        CellField::Outline => GlyphAreaField::Outline(area.outline.clone()),
        CellField::Operation(op) => GlyphAreaField::Operation(*op),
    }
}
