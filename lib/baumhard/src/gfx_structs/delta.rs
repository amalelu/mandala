// SPDX-License-Identifier: MPL-2.0

//! `Delta` — the field-set container both mutation surfaces share.
//!
//! A delta is "a set of field payloads, at most one per field kind,
//! plus a control entry naming the arithmetic that governs them".
//! That shape is identical for
//! [`GlyphArea`](crate::gfx_structs::area::GlyphArea) and
//! [`GlyphModel`](crate::gfx_structs::model::GlyphModel); only the
//! *vocabulary* differs. So the vocabulary stays two separate public
//! enums — they are the mutation seam (`CONVENTIONS.md` §B4) — while
//! the storage, the `new()` collect, the operation lookup, and the
//! additive merge live here once.
//!
//! [`DeltaGlyphArea`](crate::gfx_structs::area::DeltaGlyphArea) and
//! [`DeltaGlyphModel`](crate::gfx_structs::model::DeltaGlyphModel)
//! are aliases of `Delta<GlyphAreaField>` / `Delta<GlyphModelField>`;
//! their element-specific accessors and `Applicable` impls stay
//! beside the field vocabulary they read.

use crate::core::primitives::{ApplyOperation, Discriminated};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::ops::Add;

/// A field vocabulary that a [`Delta`] can carry.
///
/// Implementing it is two lines beyond the derives: name the
/// discriminant of the control variant, and unwrap that variant's
/// payload. Everything else — the tag itself, `same_type`, the map
/// key — comes from `#[derive(EnumDiscriminants)]` via
/// [`Discriminated`], so adding a field variant is a change to the
/// enum plus a branch in the element's apply path, and nothing else.
pub trait DeltaField: Discriminated + Clone + Debug + PartialEq {
    /// Tag of the control variant carrying the delta-wide
    /// [`ApplyOperation`] (`GlyphAreaField::Operation` /
    /// `GlyphModelField::Operation`). Used as the map key for the
    /// operation lookup, so it must match what
    /// [`Self::as_operation`] recognizes.
    const OPERATION_KEY: <Self as Discriminated>::Discriminant;

    /// The [`ApplyOperation`] this field carries when it *is* the
    /// control variant, `None` for every payload-carrying variant.
    /// O(1), no allocation.
    fn as_operation(&self) -> Option<ApplyOperation>;
}

/// A set of field deltas targeting one element, keyed by field tag so
/// at most one payload per field kind survives construction.
///
/// # Costs
///
/// One `FxHashMap` allocation per delta. Lookups are O(1) on a
/// fieldless `Copy` key — no payload hashing, no string compare.
/// Applying a delta is O(k) in the number of entries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(bound(
    serialize = "F: Serialize, <F as Discriminated>::Discriminant: Serialize",
    deserialize = "F: Deserialize<'de>, <F as Discriminated>::Discriminant: Deserialize<'de>"
))]
#[serde(deny_unknown_fields)]
pub struct Delta<F: DeltaField> {
    /// One entry per touched field kind, keyed by the field's tag.
    /// The [`DeltaField::OPERATION_KEY`] entry is a sibling of the
    /// payload entries, not a wrapper around them: it names the
    /// arithmetic (`Assign` / `Add` / `Subtract` / …) the element's
    /// apply path uses for all of them.
    pub fields: FxHashMap<<F as Discriminated>::Discriminant, F>,
}

impl<F: DeltaField> Delta<F> {
    /// Collect field payloads into a delta, keyed by tag. Duplicate
    /// variants collapse to the last one in `fields`. O(n) over
    /// `fields`; one map allocation, no payload clones (the vector is
    /// consumed).
    pub fn new(fields: Vec<F>) -> Self {
        let mut field_map = FxHashMap::default();
        for field in fields {
            field_map.insert(field.variant(), field);
        }
        Delta { fields: field_map }
    }

    /// Borrow the payload stored under `key`, if the delta touches
    /// that field. O(1); the per-field accessors on
    /// [`DeltaGlyphArea`](crate::gfx_structs::area::DeltaGlyphArea)
    /// and [`DeltaGlyphModel`](crate::gfx_structs::model::DeltaGlyphModel)
    /// are thin `match`es over this.
    #[inline]
    pub fn get(&self, key: <F as Discriminated>::Discriminant) -> Option<&F> {
        self.fields.get(&key)
    }

    /// Whether the delta carries a payload for `key`. O(1).
    #[inline]
    pub fn touches(&self, key: <F as Discriminated>::Discriminant) -> bool {
        self.fields.contains_key(&key)
    }

    /// The arithmetic mode governing every payload in this delta, or
    /// [`ApplyOperation::Noop`] when no control entry was supplied —
    /// a delta with no stated operation must not silently assign.
    /// O(1).
    #[inline]
    pub fn operation_variant(&self) -> ApplyOperation {
        self.fields
            .get(&F::OPERATION_KEY)
            .and_then(F::as_operation)
            .unwrap_or(ApplyOperation::Noop)
    }
}

/// Merge two deltas into one. Fields present on both sides are
/// combined through the vocabulary's own `Add` (which decides, per
/// field, whether "adding" means arithmetic, concatenation, or
/// last-wins); fields present on one side carry over untouched.
///
/// Cost: O(lhs + rhs) map operations and no payload clones — both
/// sides are consumed.
impl<F: DeltaField + Add<Output = F>> Add for Delta<F> {
    type Output = Delta<F>;

    fn add(self, rhs: Self) -> Self::Output {
        let mut fields = self.fields;
        for (key, value) in rhs.fields {
            match fields.remove(&key) {
                Some(existing) => fields.insert(key, existing + value),
                None => fields.insert(key, value),
            };
        }
        Delta { fields }
    }
}
