//! `KeybindConfig` — the user-editable config struct + JSON loader +
//! `resolve()` step that produces the runtime `ResolvedKeybinds` table.

use log::warn;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::action::Action;
use super::bind::KeyBind;
use super::resolved::ResolvedKeybinds;

/// The raw, user-editable config. Every field is a list of binding strings
/// so users can assign multiple keys to the same action (e.g. Ctrl+Z and
/// the Undo key both mapped to `Undo`). Fields default via serde so a
/// partial config only has to mention the actions the user wants to
/// override.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeybindConfig {
    pub undo: Vec<String>,
    pub enter_reparent_mode: Vec<String>,
    pub enter_connect_mode: Vec<String>,
    pub delete_selection: Vec<String>,
    pub cancel_mode: Vec<String>,
    pub create_orphan_node: Vec<String>,
    pub orphan_selection: Vec<String>,
    pub edit_selection: Vec<String>,
    pub edit_selection_clean: Vec<String>,
    pub open_console: Vec<String>,
    pub save_document: Vec<String>,
    /// Font family name for the console overlay. Passed verbatim to
    /// cosmic-text's `Family::Name`. Empty means "use the default
    /// fallback chain", which is usually what you want unless you've
    /// embedded a specific font.
    pub console_font: String,
    /// Font size in pixels for the console overlay. The whole frame
    /// scales with this value.
    pub console_font_size: f32,
    /// Map of key combo → custom mutation id. When the combo is
    /// pressed and no built-in `Action` matches, the app looks up
    /// the id in the merged mutation registry and applies it on the
    /// currently-selected single node. Populated by hand in
    /// `keybinds.json` or via the `mutate bind` console command
    /// (which persists to a dedicated overlay file — see
    /// `console::bindings_overlay`).
    pub custom_mutation_bindings: HashMap<String, String>,
}

impl Default for KeybindConfig {
    fn default() -> Self {
        KeybindConfig {
            undo: vec!["Ctrl+Z".into(), "Undo".into()],
            enter_reparent_mode: vec!["Ctrl+P".into()],
            enter_connect_mode: vec!["Ctrl+D".into()],
            delete_selection: vec!["Delete".into()],
            cancel_mode: vec!["Escape".into()],
            create_orphan_node: vec!["Ctrl+N".into()],
            orphan_selection: vec!["Ctrl+O".into()],
            edit_selection: vec!["Enter".into()],
            edit_selection_clean: vec!["Backspace".into()],
            open_console: vec!["/".into()],
            save_document: vec!["Ctrl+S".into()],
            console_font: String::new(),
            console_font_size: 16.0,
            custom_mutation_bindings: HashMap::new(),
        }
    }
}

impl KeybindConfig {
    /// Parse a JSON string into a config. Missing fields fall back to
    /// defaults thanks to `#[serde(default)]` on the struct.
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("parse keybinds JSON: {}", e))
    }

    /// Parse every binding string into concrete `KeyBind` values. Any
    /// binding that fails to parse is logged and skipped so a single typo
    /// doesn't break the entire config.
    pub fn resolve(&self) -> ResolvedKeybinds {
        let mut binds: Vec<(Action, KeyBind)> = Vec::new();
        let sets = [
            (Action::Undo, &self.undo),
            (Action::EnterReparentMode, &self.enter_reparent_mode),
            (Action::EnterConnectMode, &self.enter_connect_mode),
            (Action::DeleteSelection, &self.delete_selection),
            (Action::CancelMode, &self.cancel_mode),
            (Action::CreateOrphanNode, &self.create_orphan_node),
            (Action::OrphanSelection, &self.orphan_selection),
            (Action::EditSelection, &self.edit_selection),
            (Action::EditSelectionClean, &self.edit_selection_clean),
            (Action::OpenConsole, &self.open_console),
            (Action::SaveDocument, &self.save_document),
        ];
        for (action, strings) in sets {
            for s in strings {
                match KeyBind::parse(s) {
                    Ok(k) => binds.push((action, k)),
                    Err(e) => warn!("skipping invalid keybind '{}': {}", s, e),
                }
            }
        }
        let mut custom_binds: Vec<(KeyBind, String)> = Vec::new();
        for (combo, mutation_id) in &self.custom_mutation_bindings {
            match KeyBind::parse(combo) {
                Ok(k) => custom_binds.push((k, mutation_id.clone())),
                Err(e) => warn!(
                    "skipping invalid custom_mutation_binding '{}': {}",
                    combo, e
                ),
            }
        }

        ResolvedKeybinds::new(
            binds,
            custom_binds,
            self.console_font.clone(),
            self.console_font_size.max(4.0),
        )
    }
}
