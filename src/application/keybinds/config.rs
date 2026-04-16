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
    // ── Document-level (global) ──────────────────────────────────
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
    pub copy: Vec<String>,
    pub paste: Vec<String>,
    pub cut: Vec<String>,

    // ── Console ──────────────────────────────────────────────────
    pub console_close: Vec<String>,
    pub console_submit: Vec<String>,
    pub console_tab_complete: Vec<String>,
    pub console_history_up: Vec<String>,
    pub console_history_down: Vec<String>,
    pub console_cursor_left: Vec<String>,
    pub console_cursor_right: Vec<String>,
    pub console_cursor_home: Vec<String>,
    pub console_cursor_end: Vec<String>,
    pub console_delete_back: Vec<String>,
    pub console_delete_forward: Vec<String>,
    pub console_insert_space: Vec<String>,
    pub console_clear_line: Vec<String>,
    pub console_jump_start: Vec<String>,
    pub console_jump_end: Vec<String>,
    pub console_kill_to_start: Vec<String>,
    pub console_kill_word: Vec<String>,

    // ── Color Picker ─────────────────────────────────────────────
    pub picker_cancel: Vec<String>,
    pub picker_commit: Vec<String>,
    pub picker_nudge_hue_down: Vec<String>,
    pub picker_nudge_hue_up: Vec<String>,
    pub picker_nudge_sat_down: Vec<String>,
    pub picker_nudge_sat_up: Vec<String>,
    pub picker_nudge_val_down: Vec<String>,
    pub picker_nudge_val_up: Vec<String>,

    // ── Label Editor ─────────────────────────────────────────────
    pub label_edit_cancel: Vec<String>,
    pub label_edit_commit: Vec<String>,

    // ── Text Editor ──────────────────────────────────────────────
    pub text_edit_cancel: Vec<String>,

    // ── Style / metadata ─────────────────────────────────────────
    /// Font family name for the console overlay.
    pub console_font: String,
    /// Font size in pixels for the console overlay.
    pub console_font_size: f32,
    /// Map of key combo → custom mutation id.
    pub custom_mutation_bindings: HashMap<String, String>,
}

impl Default for KeybindConfig {
    fn default() -> Self {
        KeybindConfig {
            // Document-level
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
            copy: vec!["Ctrl+C".into(), "Copy".into()],
            paste: vec!["Ctrl+V".into(), "Paste".into()],
            cut: vec!["Ctrl+X".into(), "Cut".into()],

            // Console
            console_close: vec!["Escape".into()],
            console_submit: vec!["Enter".into()],
            console_tab_complete: vec!["Tab".into()],
            console_history_up: vec!["ArrowUp".into(), "Up".into()],
            console_history_down: vec!["ArrowDown".into(), "Down".into()],
            console_cursor_left: vec!["ArrowLeft".into(), "Left".into()],
            console_cursor_right: vec!["ArrowRight".into(), "Right".into()],
            console_cursor_home: vec!["Home".into()],
            console_cursor_end: vec!["End".into()],
            console_delete_back: vec!["Backspace".into()],
            console_delete_forward: vec!["Delete".into()],
            console_insert_space: vec!["Space".into()],
            console_clear_line: vec!["Ctrl+C".into()],
            console_jump_start: vec!["Ctrl+A".into()],
            console_jump_end: vec!["Ctrl+E".into()],
            console_kill_to_start: vec!["Ctrl+U".into()],
            console_kill_word: vec!["Ctrl+W".into()],

            // Color Picker
            picker_cancel: vec!["Escape".into()],
            picker_commit: vec!["Enter".into()],
            picker_nudge_hue_down: vec!["h".into()],
            picker_nudge_hue_up: vec!["Shift+h".into()],
            picker_nudge_sat_down: vec!["s".into()],
            picker_nudge_sat_up: vec!["Shift+s".into()],
            picker_nudge_val_down: vec!["v".into()],
            picker_nudge_val_up: vec!["Shift+v".into()],

            // Label Editor
            label_edit_cancel: vec!["Escape".into()],
            label_edit_commit: vec!["Enter".into()],

            // Text Editor
            text_edit_cancel: vec!["Escape".into()],

            // Style / metadata
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
        let sets: &[(Action, &Vec<String>)] = &[
            // Document-level
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
            (Action::Copy, &self.copy),
            (Action::Paste, &self.paste),
            (Action::Cut, &self.cut),
            // Console
            (Action::ConsoleClose, &self.console_close),
            (Action::ConsoleSubmit, &self.console_submit),
            (Action::ConsoleTabComplete, &self.console_tab_complete),
            (Action::ConsoleHistoryUp, &self.console_history_up),
            (Action::ConsoleHistoryDown, &self.console_history_down),
            (Action::ConsoleCursorLeft, &self.console_cursor_left),
            (Action::ConsoleCursorRight, &self.console_cursor_right),
            (Action::ConsoleCursorHome, &self.console_cursor_home),
            (Action::ConsoleCursorEnd, &self.console_cursor_end),
            (Action::ConsoleDeleteBack, &self.console_delete_back),
            (Action::ConsoleDeleteForward, &self.console_delete_forward),
            (Action::ConsoleInsertSpace, &self.console_insert_space),
            (Action::ConsoleClearLine, &self.console_clear_line),
            (Action::ConsoleJumpStart, &self.console_jump_start),
            (Action::ConsoleJumpEnd, &self.console_jump_end),
            (Action::ConsoleKillToStart, &self.console_kill_to_start),
            (Action::ConsoleKillWord, &self.console_kill_word),
            // Color Picker
            (Action::PickerCancel, &self.picker_cancel),
            (Action::PickerCommit, &self.picker_commit),
            (Action::PickerNudgeHueDown, &self.picker_nudge_hue_down),
            (Action::PickerNudgeHueUp, &self.picker_nudge_hue_up),
            (Action::PickerNudgeSatDown, &self.picker_nudge_sat_down),
            (Action::PickerNudgeSatUp, &self.picker_nudge_sat_up),
            (Action::PickerNudgeValDown, &self.picker_nudge_val_down),
            (Action::PickerNudgeValUp, &self.picker_nudge_val_up),
            // Label Editor
            (Action::LabelEditCancel, &self.label_edit_cancel),
            (Action::LabelEditCommit, &self.label_edit_commit),
            // Text Editor
            (Action::TextEditCancel, &self.text_edit_cancel),
        ];
        for (action, strings) in sets {
            for s in *strings {
                match KeyBind::parse(s) {
                    Ok(k) => binds.push((*action, k)),
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
