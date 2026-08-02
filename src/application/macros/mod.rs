// SPDX-License-Identifier: MPL-2.0

//! Macro types and registry.
//!
//! A macro is a named `Vec<MacroStep>` invoked atomically through
//! `crate::application::app::dispatch::dispatch_macro`. Three step
//! kinds — built-in `Action`, parameterised `CustomMutation`,
//! console-line — cover every existing user-driven entry point at the
//! application layer. Plugins, future macro-recording UI, and the
//! `keybind_bindings` resolution tier all reach the same dispatcher.
//!
//! Loading parallels the custom-mutation loader at
//! `src/application/document/mutations_loader/`: a registry is built
//! once at startup, queried by id, and bindable from `keybinds.json`
//! via the `macro_bindings: HashMap<String, String>` field on
//! `KeybindConfig`.

pub mod loader;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::application::keybinds::Action;
use crate::application::source_tier::SourceTier;

/// **Macro privilege model**, hung off the shared
/// [`SourceTier`] ladder.
///
/// The tier a macro was loaded from is also how much the macro is
/// trusted. [`MacroStep::ConsoleLine`] runs an arbitrary console
/// verb — including filesystem-touching ones (`save <path>`,
/// `open <path>`) — so to prevent a hostile shared mindmap from
/// doing arbitrary file I/O, only [`SourceTier::User`] macros may
/// contain `ConsoleLine` steps. The dispatcher rejects
/// `ConsoleLine` from `App`, `Map`, and `Inline` tiers with a
/// `warn!`. All four tiers load today on native; the gate is fully
/// active.
///
/// Document-mutating step kinds (`Action`, `CustomMutation`) gate
/// via [`SourceTier::allows_action`].
///
/// These live here rather than in `source_tier` because the policy
/// is about what a *macro step* may do, not about what a tier is.
impl SourceTier {
    /// Whether macros from this source may carry
    /// `MacroStep::ConsoleLine` steps. Only `User` macros pass —
    /// app-bundled / map-inline / node-inline macros loaded from
    /// untrusted sources cannot execute arbitrary console verbs.
    pub fn allows_console_line(self) -> bool {
        matches!(self, SourceTier::User)
    }

    /// Whether macros from this source may invoke the given Action
    /// via a `MacroStep::Action`. Symmetric with
    /// [`Self::allows_console_line`]: only `User`-tier macros can fire
    /// the destructive / I/O / clipboard-touching Actions, since
    /// other tiers may load from untrusted sources (a hostile
    /// `.mindmap.json` could otherwise bind `Action::SaveDocument`
    /// to a hotkey and overwrite the user's file the next time
    /// they press the bound key).
    ///
    /// All four tiers load today on native; the gate is fully
    /// active. The structural backstop is `Action::is_destructive`
    /// — an exhaustive `match` over `Action` that the compiler
    /// enforces. A new variant added to the `#[non_exhaustive]`
    /// `Action` enum cannot land without an explicit
    /// destructive / non-destructive classification, so this gate
    /// cannot silently widen on future variants. (The previous
    /// shape was a hand-maintained `matches!` denylist that
    /// defaulted new variants to "allowed" — a missing entry
    /// silently bypassed the gate. A real `LabelEditOnSelection`
    /// gap surfaced from that pattern.)
    pub fn allows_action(self, action: &Action) -> bool {
        if matches!(self, SourceTier::User) {
            return true;
        }
        !action.is_destructive()
    }
}

/// One step inside a macro. A macro is a sequence of these executed
/// atomically. Each variant routes through one of the application's
/// existing dispatch surfaces, so adding a new step kind is a matter
/// of forwarding to the relevant dispatcher — no new behaviour.
///
/// `#[non_exhaustive]` because new step kinds need to be reviewed
/// against the privilege gate (do they need a tier check like
/// `ConsoleLine` does?) and the `dispatch_macro` body's per-step
/// loop. Mirrors `Action`'s `#[non_exhaustive]` discipline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[non_exhaustive]
pub enum MacroStep {
    /// Run a built-in `Action` against the current `InputHandlerContext`.
    /// The dispatcher routes through `dispatch_action` exactly as if
    /// the user had pressed the action's bound key.
    Action { action: Action },
    /// Apply a custom mutation by id, against the resolved target.
    /// Forwards to the same path keybind-triggered custom mutations
    /// use (animation-aware, document-actions parity).
    CustomMutation {
        id: String,
        #[serde(default)]
        target: MacroTarget,
    },
    /// Re-parse the given line through the console parser and run it
    /// as if typed. Lets macros leverage every parameterised verb
    /// (e.g. `border preset=triple`) without needing a bespoke step
    /// kind.
    ConsoleLine { line: String },
}

/// How a `MacroStep::CustomMutation` resolves its target node id at
/// dispatch time. `CurrentSelection` matches the selection-driven
/// click-trigger path; `NodeId` lets a macro target a specific node
/// regardless of selection state (useful for "open the inbox" style
/// shortcuts).
///
/// `#[non_exhaustive]` because new target shapes (e.g. "all
/// selected nodes", "current document root") need to be reviewed
/// against the dispatcher's per-target resolution loop.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MacroTarget {
    /// Use whichever node is currently selected (must be a single
    /// selection — multi-selection or edge selections cause the step
    /// to skip).
    #[default]
    CurrentSelection,
    /// Always target the named node. Skips if the node id doesn't
    /// resolve in the current document.
    NodeId(String),
}

/// A user-defined macro: id + ordered list of steps. Loaded from
/// up to four tiers on native (App bundle / User config /
/// per-Map / per-node Inline); see [`SourceTier`] for the
/// precedence order and `loader.rs` for the parse plumbing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Macro {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<MacroStep>,
}

/// In-memory lookup table. Built once at startup from the loader's
/// merged slices, then refreshed on every document-replace.
///
/// **Shadow-stacked storage.** Each id maps to a per-tier slot
/// array indexed by `SourceTier::index`: `[App, User, Map,
/// Inline]`. Lookup walks high-to-low precedence; the first
/// non-None slot wins. Higher-tier entries SHADOW lower-tier ones
/// rather than displacing them — `clear_tier(Inline)` removes
/// only the Inline slot, and `get` / `get_with_source` then
/// resolve to whichever lower-tier entry exists. This fixes the
/// "displacement is permanent within the session" problem the
/// reviewers flagged: a User-tier `id="x"` shadowed by a Map-tier
/// `id="x"` re-emerges naturally when the document is replaced
/// (which clears the Map tier).
///
/// Within-tier collisions (e.g. two entries with `id="x"` in the
/// same Map-tier `mindmap.macros` array) still last-writer-wins —
/// only the cross-tier reveal property is new.
#[derive(Debug, Clone, Default)]
pub struct MacroRegistry {
    macros: HashMap<String, [Option<Macro>; SourceTier::COUNT]>,
}

impl MacroRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Write a macro into the slot for `source`. Returns the
    /// previous entry AT THE SAME TIER — not the displaced lower-
    /// tier entry, since lower tiers are no longer displaced.
    /// Within-tier last-writer-wins; cross-tier coexistence is
    /// preserved.
    pub fn insert(&mut self, m: Macro, source: SourceTier) -> Option<Macro> {
        let id = m.id.clone();
        let slots = self
            .macros
            .entry(id)
            .or_insert_with(|| std::array::from_fn(|_| None));
        slots[source.index()].replace(m)
    }

    /// Look up the highest-tier macro for `id`. Walks the slot
    /// array from Inline → Map → User → App and returns the first
    /// non-None entry.
    pub fn get(&self, id: &str) -> Option<&Macro> {
        let slots = self.macros.get(id)?;
        for i in (0..SourceTier::COUNT).rev() {
            if let Some(m) = &slots[i] {
                return Some(m);
            }
        }
        None
    }

    /// Look up the highest-tier macro for `id` and the tier that
    /// holds it. The dispatcher uses this pair to consult the
    /// privilege gate (`SourceTier::allows_console_line`,
    /// `allows_action`). Same walk order as `get`.
    pub fn get_with_source(&self, id: &str) -> Option<(&Macro, SourceTier)> {
        let slots = self.macros.get(id)?;
        // Walk tiers high-to-low straight off the shared ladder, so
        // a future tier extends `SourceTier::ALL` and nothing else.
        for tier in SourceTier::ALL.into_iter().rev() {
            if let Some(m) = &slots[tier.index()] {
                return Some((m, tier));
            }
        }
        None
    }

    /// Whether any tier slot holds an entry for `id`.
    pub fn contains(&self, id: &str) -> bool {
        self.macros
            .get(id)
            .map_or(false, |slots| slots.iter().any(Option::is_some))
    }

    /// Number of distinct ids with at least one tier slot occupied.
    /// An id present at multiple tiers counts once.
    pub fn len(&self) -> usize {
        self.macros
            .values()
            .filter(|slots| slots.iter().any(Option::is_some))
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate registered macro ids. An id present at multiple
    /// tiers appears once. HashMap iteration order is unspecified;
    /// sort at the call site if a stable display is needed.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.macros
            .iter()
            .filter(|(_, slots)| slots.iter().any(Option::is_some))
            .map(|(k, _)| k.as_str())
    }

    /// Clear the slot for `source` across every id. Lower-tier
    /// entries with the same id survive in their slots —
    /// `get` / `get_with_source` will resolve them once this tier
    /// is cleared. Drops the HashMap entry entirely if all four
    /// slots become None.
    ///
    /// This is the load-bearing piece of the shadow-stack design:
    /// document-replace paths can clear Map + Inline tiers without
    /// disturbing the App + User tiers loaded at startup, AND the
    /// previously-shadowed User-tier entries re-emerge naturally
    /// in subsequent lookups.
    pub fn clear_tier(&mut self, source: SourceTier) {
        let idx = source.index();
        self.macros.retain(|_, slots| {
            slots[idx] = None;
            slots.iter().any(Option::is_some)
        });
    }

    /// Bulk-insert macros at the given tier. Within-tier id
    /// collisions follow last-writer-wins (later iterator entries
    /// override earlier ones). Cross-tier entries at OTHER tiers
    /// survive in their slots — this is the shadow-stacking
    /// property.
    pub fn extend_with_tier<I: IntoIterator<Item = Macro>>(&mut self, macros: I, source: SourceTier) {
        for m in macros {
            self.insert(m, source);
        }
    }
}

mod tests;
