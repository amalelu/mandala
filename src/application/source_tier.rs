// SPDX-License-Identifier: MPL-2.0

//! The provenance ladder shared by every id-keyed registry in the
//! application.
//!
//! Custom mutations and macros are both looked up by string id and
//! both accept definitions from four places: the binary's own
//! bundle, the user's config directory, the currently-open document,
//! and an individual node. Those four places form one ladder, and
//! the ladder means two things at once:
//!
//! - **Precedence.** Later tiers override earlier ones on an id
//!   collision, so `App < User < Map < Inline` — the enum derives
//!   `Ord` in that order and the derive is the contract, not a
//!   comment.
//! - **Trust.** The ladder is also a trust ranking. `App` content
//!   ships with the binary and `User` content is the user's own
//!   file, but `Map` and `Inline` content arrives inside a
//!   `.mindmap.json` that may have been shared by a stranger. The
//!   macro dispatcher's privilege gates key off exactly this
//!   distinction; they live next to the macro types in
//!   `crate::application::macros` rather than here, because the
//!   policy is about what a *macro step* may do, not about what a
//!   tier is.
//!
//! Both registries used to declare their own four-variant copy of
//! this enum, kept in step by a "mirrors the other one" comment. One
//! enum, one order, one test.

/// Which layer a registry entry — a custom mutation, a macro — was
/// loaded from.
///
/// Variants are declared in ascending precedence and the derived
/// `Ord` follows that declaration, so `SourceTier::App <
/// SourceTier::Inline` is a compile-checked way to ask "does this
/// tier lose?". [`SourceTier::ALL`] lists them in the same ascending
/// order.
///
/// `#[non_exhaustive]` because a fifth tier (a plugin bundle, say)
/// has to be placed deliberately in the ladder rather than appended
/// by reflex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum SourceTier {
    /// Shipped with the binary — `assets/mutations/application.json`
    /// and `assets/macros/application.json`, both embedded with
    /// `include_str!`. Lowest precedence, and the only tier whose
    /// content cannot have been modified after the build.
    App,
    /// Loaded from the user's own config file
    /// (`$XDG_CONFIG_HOME/mandala/<name>.json` on native; the
    /// `?<name>=` query param or `localStorage` on WASM). The user
    /// owns the file, so this is the trusted tier for privileged
    /// operations.
    User,
    /// Declared in the currently-loaded document — `mindmap.macros`
    /// or `mindmap.custom_mutations`. Refreshed on every document
    /// load, and untrusted: a shared `.mindmap.json` is a stranger's
    /// file.
    Map,
    /// Declared on an individual node — `MindNode.inline_macros` or
    /// `MindNode.inline_mutations`. Highest precedence, and
    /// untrusted for the same reason as [`SourceTier::Map`].
    Inline,
}

impl SourceTier {
    /// Every tier, in ascending precedence. This is the canonical
    /// order: registries index their per-tier slot arrays by
    /// [`SourceTier::index`], which agrees with this list, and
    /// resolution walks it in reverse.
    pub const ALL: [SourceTier; 4] = [
        SourceTier::App,
        SourceTier::User,
        SourceTier::Map,
        SourceTier::Inline,
    ];

    /// Number of tiers. Registries size their per-id slot arrays
    /// against this.
    pub const COUNT: usize = Self::ALL.len();

    /// Index into a per-tier slot array. Higher index = higher
    /// precedence, matching [`SourceTier::ALL`]'s order.
    ///
    /// Written as an exhaustive `match` rather than an `as usize`
    /// cast so that adding a variant is a compile error here instead
    /// of a silent renumbering of every stored slot.
    pub(crate) const fn index(self) -> usize {
        match self {
            SourceTier::App => 0,
            SourceTier::User => 1,
            SourceTier::Map => 2,
            SourceTier::Inline => 3,
        }
    }

    /// Lowercase one-word name, as the console prints it (`mutation
    /// help <id>` reports which tier won an id).
    pub fn label(self) -> &'static str {
        // `#[non_exhaustive]` forces a wildcard arm even though
        // every variant is matched; it is the forward-compat seam
        // for a future tier and is unreachable today.
        #[allow(unreachable_patterns)]
        match self {
            SourceTier::App => "app",
            SourceTier::User => "user",
            SourceTier::Map => "map",
            SourceTier::Inline => "inline",
            _ => "(unknown source)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The user tier outranks the built-in tier — a user's
    /// `macros.json` / `mutations.json` entry overrides the bundled
    /// one with the same id, never the other way around. This is the
    /// single most user-visible consequence of the ladder.
    #[test]
    fn test_user_tier_outranks_app_tier() {
        assert!(SourceTier::App < SourceTier::User);
        assert!(SourceTier::App.index() < SourceTier::User.index());
    }

    /// Document-supplied tiers outrank the user's config, and a
    /// per-node declaration outranks the document's.
    #[test]
    fn test_document_tiers_outrank_user_tier() {
        assert!(SourceTier::User < SourceTier::Map);
        assert!(SourceTier::Map < SourceTier::Inline);
    }

    /// The load-bearing pin on the ladder itself: `ALL` is in
    /// ascending precedence, `index()` agrees with it position for
    /// position, and `Ord` agrees with both.
    ///
    /// `Ord` is what precedence comparisons use; `index()` is what
    /// the macro registry's slot array uses. If they ever disagreed,
    /// a macro would resolve from a different tier than `mutation
    /// help` reports.
    ///
    /// Note what this pins that a literal `assert_eq!(ALL, [App,
    /// User, Map, Inline])` would not: because `index()` is a
    /// hand-written exhaustive `match` and `Ord` is derived from the
    /// *declaration* order, reordering the variants in the `enum`
    /// turns this test red, whereas a literal spelled with the same
    /// variant names would reorder along with it and stay green. The
    /// first loop also proves `ALL` lists all four tiers exactly once
    /// — four distinct indices across four slots leaves no room for a
    /// duplicate or an omission.
    #[test]
    fn test_tier_ord_agrees_with_index() {
        for (i, tier) in SourceTier::ALL.iter().enumerate() {
            assert_eq!(tier.index(), i, "{:?} must index at {}", tier, i);
        }
        for pair in SourceTier::ALL.windows(2) {
            assert!(pair[0] < pair[1], "{:?} must rank below {:?}", pair[0], pair[1]);
        }
    }

    /// `COUNT` sizes the registries' slot arrays; a tier missing
    /// from `ALL` would silently shrink them.
    #[test]
    fn test_tier_count_matches_all() {
        assert_eq!(SourceTier::COUNT, 4);
        assert_eq!(SourceTier::COUNT, SourceTier::ALL.len());
    }

    /// Every tier prints a distinct, non-empty label — the console
    /// readout must never show two tiers under one name.
    #[test]
    fn test_tier_labels_are_distinct_and_non_empty() {
        let mut seen: Vec<&str> = Vec::new();
        for tier in SourceTier::ALL {
            let label = tier.label();
            assert!(!label.is_empty(), "{:?} has no label", tier);
            assert!(!seen.contains(&label), "label '{}' is used twice", label);
            seen.push(label);
        }
    }
}
