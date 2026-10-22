// SPDX-License-Identifier: MPL-2.0

//! Triple-state edit primitive (`Keep` / `Clear` / `Set`) and the
//! field-level fold helpers that consume it. These are the
//! building blocks every node-style setter routes its
//! per-field "leave alone vs clear vs write" decision through —
//! the shared shape is what lets a single console verb stage a
//! mix of `font=off` / `padding=4` / (untouched preset) edits and
//! hand the bundle to one atomic setter.

/// Triple-state edit on an `Option<T>` field. The three variants
/// distinguish "leave alone" from "explicitly clear an existing
/// override" from "set to a concrete value". Used by the
/// `BorderConfigEdits` bundle (every per-field slot is one of these)
/// and by the zoom-visibility setters (where a console line like
/// `zoom min=1.5 max=unset` translates each kv into an
/// `OptionEdit<f32>` so a single setter call handles both sides
/// atomically). The shared shape is what makes the console verbs'
/// `palette=off` / `font=off` / `min=unset` syntax possible —
/// without `Clear`, callers couldn't distinguish "the user didn't
/// mention this field" from "the user wants this field cleared".
///
/// `Keep` is the default so [`super::border::BorderConfigEdits`]'s
/// `#[derive(Default)]` builds the no-op edit set, and the console
/// verb only fills in the keys the user actually typed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum OptionEdit<T> {
    /// No edit — leave the model field at its current value.
    #[default]
    Keep,
    /// Drop the per-node override; the resolver cascade falls
    /// through to the canvas-level default or hardcoded floor.
    Clear,
    /// Write this concrete value to the field.
    Set(T),
}

impl<T: Clone> OptionEdit<T> {
    /// Fold this edit against `current`, yielding the new
    /// `Option<T>` value. Pure, O(1). The single canonical
    /// implementation of the Keep/Clear/Set semantics — every
    /// consumer (`zoom_bounds` setters today, future
    /// border-config writes when the bespoke
    /// `apply_option_edit` helper folds in) goes through this
    /// method instead of re-matching the three variants.
    pub fn apply(self, current: Option<T>) -> Option<T> {
        match self {
            OptionEdit::Keep => current,
            OptionEdit::Clear => None,
            OptionEdit::Set(v) => Some(v),
        }
    }
}

/// Apply a `OptionEdit<T>` to an `Option<U>` slot, with `to_target`
/// projecting `T → U` for the value-write path. Returns `true` when the
/// slot actually changed. The four `font / color / color_palette /
/// color_palette_field` arms in `apply_border_edits` were structurally
/// identical (Set→write-if-different, Clear→None-if-some, Keep→no-op);
/// they collapse to one call each through this helper. The `to_target`
/// closure exists because `color_palette_field` writes a `String`-typed
/// slot from a `PaletteField` enum, so the projection isn't always
/// `clone()`.
pub(super) fn apply_option_edit<T, U>(
    edit: &OptionEdit<T>,
    slot: &mut Option<U>,
    to_target: impl FnOnce(&T) -> U,
) -> bool
where
    U: PartialEq,
{
    match edit {
        OptionEdit::Set(v) => {
            let new = to_target(v);
            if slot.as_ref() != Some(&new) {
                *slot = Some(new);
                return true;
            }
        }
        OptionEdit::Clear => {
            if slot.is_some() {
                *slot = None;
                return true;
            }
        }
        OptionEdit::Keep => {}
    }
    false
}

/// `String`-specialised value-set used by the side / corner glyph
/// fields: only the `Set` arm writes; `Clear` and `Keep` are no-ops
/// because the eight glyph slots in
/// [`baumhard::mindmap::model::CustomBorderGlyphs`] are
/// non-`Option<String>` (the schema stores hardcoded fallbacks
/// rather than tri-state overrides).
pub(super) fn apply_string_set(edit: &OptionEdit<String>, slot: &mut String) -> bool {
    match edit {
        OptionEdit::Set(v) if slot != v => {
            *slot = v.clone();
            true
        }
        _ => false,
    }
}
