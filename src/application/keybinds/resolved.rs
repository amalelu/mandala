//! `ResolvedKeybinds` — the runtime lookup table the event loop calls
//! into. Built once via `KeybindConfig::resolve`, then queried per
//! input event.

use std::collections::HashMap;

use super::action::Action;
use super::bind::KeyBind;
use super::context::InputContext;

/// The resolved form of a `KeybindConfig`: a flat list of `(Action,
/// KeyBind)` pairs. Lookup is linear — the list is small enough
/// (under 50 entries) that a hash map would only add overhead.
#[derive(Debug, Clone)]
pub struct ResolvedKeybinds {
    binds: Vec<(Action, KeyBind)>,
    /// Parsed `(KeyBind, mutation_id)` pairs from
    /// `KeybindConfig::custom_mutation_bindings`. Checked after the
    /// built-in `action_for` lookup in the event loop — a key combo
    /// bound to both a built-in action and a custom mutation
    /// resolves to the built-in action (action_for runs first).
    custom_binds: Vec<(KeyBind, String)>,
    /// Console font family. Empty means "use cosmic-text default".
    pub console_font: String,
    /// Console overlay font size in pixels.
    pub console_font_size: f32,
}

impl ResolvedKeybinds {
    /// Construct a resolved table — called from `KeybindConfig::resolve`,
    /// which owns the validation + parsing of binding strings.
    pub(super) fn new(
        binds: Vec<(Action, KeyBind)>,
        custom_binds: Vec<(KeyBind, String)>,
        console_font: String,
        console_font_size: f32,
    ) -> Self {
        Self {
            binds,
            custom_binds,
            console_font,
            console_font_size,
        }
    }

    /// Return the action bound to the given key event, if any. The caller
    /// passes the normalized key name (see `normalize_key_name`) and the
    /// current modifier state. Searches all actions regardless of context —
    /// use `action_for_context` for context-aware resolution.
    pub fn action_for(&self, key: &str, ctrl: bool, shift: bool, alt: bool) -> Option<Action> {
        for (action, bind) in &self.binds {
            if bind.matches(key, ctrl, shift, alt) {
                return Some(*action);
            }
        }
        None
    }

    /// Resolve an action for a key event within a given input context.
    /// Tries context-specific actions first. If the context allows
    /// fallthrough and no context-specific action matched, tries
    /// the parent context.
    pub fn action_for_context(
        &self,
        context: InputContext,
        key: &str,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> Option<Action> {
        for (action, bind) in &self.binds {
            if bind.matches(key, ctrl, shift, alt) && action.context() == context {
                return Some(*action);
            }
        }
        if context.falls_through() {
            let parent = context.parent();
            for (action, bind) in &self.binds {
                if bind.matches(key, ctrl, shift, alt) && action.context() == parent {
                    return Some(*action);
                }
            }
        }
        None
    }

    /// Returns true if the given key event is bound to the given action.
    /// Convenience for the event loop.
    pub fn is(&self, action: Action, key: &str, ctrl: bool, shift: bool, alt: bool) -> bool {
        self.action_for(key, ctrl, shift, alt) == Some(action)
    }

    /// Return the custom-mutation id bound to the given key event,
    /// if any. Called after `action_for` returns `None` — built-in
    /// actions win on a collision.
    pub fn custom_mutation_for(
        &self,
        key: &str,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> Option<&str> {
        for (bind, id) in &self.custom_binds {
            if bind.matches(key, ctrl, shift, alt) {
                return Some(id.as_str());
            }
        }
        None
    }

    /// Set or replace a custom-mutation binding at runtime. Returns
    /// the previous mutation id bound to the same combo, if any.
    /// The `combo_string` is re-parsed through `KeyBind::parse` so
    /// invalid inputs are rejected uniformly with the resolve-time
    /// path.
    pub fn set_custom_mutation_binding(
        &mut self,
        combo_string: &str,
        mutation_id: String,
    ) -> Result<Option<String>, String> {
        let bind = KeyBind::parse(combo_string)?;
        let mut prev = None;
        self.custom_binds.retain(|(b, id)| {
            if b == &bind {
                prev = Some(id.clone());
                false
            } else {
                true
            }
        });
        self.custom_binds.push((bind, mutation_id));
        Ok(prev)
    }

    /// Remove the custom-mutation binding for the given combo.
    /// Returns the removed mutation id, if one was bound.
    pub fn remove_custom_mutation_binding(
        &mut self,
        combo_string: &str,
    ) -> Result<Option<String>, String> {
        let bind = KeyBind::parse(combo_string)?;
        let mut prev = None;
        self.custom_binds.retain(|(b, id)| {
            if b == &bind {
                prev = Some(id.clone());
                false
            } else {
                true
            }
        });
        Ok(prev)
    }

    /// Snapshot the current custom-mutation bindings as a `HashMap`
    /// of `combo_string → mutation_id` for persistence. Inverse of
    /// the resolve-time parse step — used when writing the overlay
    /// file.
    pub fn custom_mutation_binding_snapshot(&self) -> HashMap<String, String> {
        self.custom_binds
            .iter()
            .map(|(b, id)| (b.to_binding_string(), id.clone()))
            .collect()
    }
}
