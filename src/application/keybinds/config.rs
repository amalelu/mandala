// SPDX-License-Identifier: MPL-2.0

//! `KeybindConfig` — the user-editable config struct + JSON loader +
//! `resolve()` step that produces the runtime `ResolvedKeybinds` table.

use log::warn;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::action::Action;
use super::bind::KeyBind;
use super::resolved::ResolvedKeybinds;

/// One binding for a parametric Action — a key combo plus the
/// positional payload args the resolve step feeds into the
/// variant's payload. Free-form `String` args; typed validation
/// happens in the dispatch arm (parse failures emit a warn-log and
/// the dispatch returns `Handled` as a best-effort no-op — Action
/// arms have no scrollback surface).
///
/// Args are positional rather than keyed so the same shape covers
/// single-payload (1 arg, e.g. `set_edge_body_glyph(["dash"])`),
/// from/to (2 args, e.g. `set_edge_anchor(["top", "auto"])`), and
/// field/value (2 args) variants. Per-variant arg counts are
/// documented next to each `Action` definition; a binding with the
/// wrong count emits a warn-log and is skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParametricBinding {
    pub combo: String,
    #[serde(default)]
    pub args: Vec<String>,
}

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
    /// Enter resize mode on the current selection — Batch 2 of
    /// `SECTIONS_BORDERS_RESIZE_PLAN.md`. Default `r`.
    pub enter_resize_mode: Vec<String>,
    /// Start a corner-anchored fast-resize gesture from a
    /// right-button drag. Default: `["Ctrl+RightDrag"]`. Lookup
    /// goes through `action_for_gesture` so the standard
    /// modifier-fallback applies — a user who clears the default
    /// and binds bare `["RightDrag"]` gets fast-resize on plain
    /// right-drag, no Ctrl required.
    ///
    /// Parametric on the keybind side (a parametric binding has
    /// no payload arms here — `Action::FastResizeStart` itself
    /// takes no payload), but kept on the regular `Vec<String>`
    /// surface for ergonomic JSON. Mirrors the
    /// `enter_resize_mode` shape.
    pub fast_resize_start: Vec<String>,
    pub delete_selection: Vec<String>,
    pub exit_mode: Vec<String>,
    pub create_orphan_node: Vec<String>,
    pub orphan_selection: Vec<String>,
    pub edit_selection: Vec<String>,
    pub edit_selection_clean: Vec<String>,
    /// Enter NodeEdit mode (Batch 3 of
    /// `SECTIONS_BORDERS_RESIZE_PLAN.md`). The umbrella `Enter`
    /// keybind dispatches `EditSelection` first (which fans out
    /// here for node-bearing selections), so users normally don't
    /// rebind `enter_node_edit` — but the explicit hook is here
    /// for macros and future GUI buttons. Default unbound.
    pub enter_node_edit: Vec<String>,
    pub enter_node_edit_clean: Vec<String>,
    /// Enter SectionEdit mode (text editor) on the active section
    /// while in NodeEdit mode. Default `Enter` at the
    /// `InputContext::NodeEdit` level — same key as `EditSelection`
    /// at Document, since the contexts don't overlap (the modal
    /// cascade picks NodeEdit over Document when NodeEdit is active).
    pub enter_section_edit: Vec<String>,
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
    pub console_scroll_up: Vec<String>,
    pub console_scroll_down: Vec<String>,
    pub console_scroll_page_up: Vec<String>,
    pub console_scroll_page_down: Vec<String>,
    pub console_scroll_end: Vec<String>,
    pub console_scroll_home: Vec<String>,

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

    // ── Border Preview ───────────────────────────────────────────
    /// Discard the active border preview.
    ///
    /// **Default: unbound** — but the user-visible Esc behavior
    /// is "if a preview is active, cancel it; otherwise exit the
    /// current mode" because `Action::ExitMode` (the default Esc
    /// binding) calls `cancel_border_preview()` first and
    /// short-circuits if a preview was canceled. See
    /// `app/dispatch/action_core.rs` for the chain. The chain
    /// lives in `ExitMode`'s body rather than as a separate
    /// keybind because the resolver maps `(context, key) →
    /// Action` deterministically and can't fall through from
    /// one Action to another based on document state.
    ///
    /// Bind this entry to opt out of the chain — e.g. to put
    /// preview-cancel on a different key while leaving `Escape`
    /// bound to plain `ExitMode`. The verb path `border preview
    /// cancel` is the primary surface and works regardless.
    pub cancel_border_preview: Vec<String>,
    /// Commit the active border preview through the matching
    /// committing setter. No default binding — users opt in.
    /// Unlike `cancel_border_preview`, commit isn't part of any
    /// implicit chain — the `border preview commit` verb is the
    /// primary surface and a binding here is purely opt-in
    /// muscle-memory.
    pub commit_border_preview: Vec<String>,

    // ── Mouse-gesture Actions ────────────────────────────────────
    pub double_click_activate: Vec<String>,
    pub create_orphan_node_and_edit: Vec<String>,
    pub pan_canvas: Vec<String>,

    // ── Navigation / camera ──────────────────────────────────────
    pub zoom_in: Vec<String>,
    pub zoom_out: Vec<String>,
    pub zoom_reset: Vec<String>,
    pub zoom_fit: Vec<String>,
    pub pan_camera_north: Vec<String>,
    pub pan_camera_south: Vec<String>,
    pub pan_camera_east: Vec<String>,
    pub pan_camera_west: Vec<String>,
    pub center_on_selection: Vec<String>,
    pub jump_to_root: Vec<String>,

    // ── Selection ────────────────────────────────────────────────
    pub select_all: Vec<String>,
    pub deselect_all: Vec<String>,
    pub invert_selection: Vec<String>,
    pub select_parent: Vec<String>,
    pub select_child: Vec<String>,
    pub select_next_sibling: Vec<String>,
    pub select_prev_sibling: Vec<String>,

    // ── TextEdit cursor primitives ──────────────────────────────
    pub text_edit_cursor_left: Vec<String>,
    pub text_edit_cursor_right: Vec<String>,
    pub text_edit_cursor_up: Vec<String>,
    pub text_edit_cursor_down: Vec<String>,
    pub text_edit_cursor_home: Vec<String>,
    pub text_edit_cursor_end: Vec<String>,
    pub text_edit_cursor_left_select: Vec<String>,
    pub text_edit_cursor_right_select: Vec<String>,
    pub text_edit_cursor_up_select: Vec<String>,
    pub text_edit_cursor_down_select: Vec<String>,
    pub text_edit_cursor_home_select: Vec<String>,
    pub text_edit_cursor_end_select: Vec<String>,
    pub text_edit_word_left: Vec<String>,
    pub text_edit_word_right: Vec<String>,
    pub text_edit_delete_back: Vec<String>,
    pub text_edit_delete_forward: Vec<String>,
    pub text_edit_delete_word_back: Vec<String>,
    pub text_edit_delete_word_forward: Vec<String>,
    pub text_edit_commit: Vec<String>,

    // ── LabelEdit cursor primitives ─────────────────────────────
    pub label_edit_cursor_left: Vec<String>,
    pub label_edit_cursor_right: Vec<String>,
    pub label_edit_cursor_home: Vec<String>,
    pub label_edit_cursor_end: Vec<String>,
    pub label_edit_delete_back: Vec<String>,
    pub label_edit_delete_forward: Vec<String>,

    // ── Console-verb Actions ────────────────────────────────────
    pub open_color_picker: Vec<String>,
    pub close_color_picker: Vec<String>,
    pub label_edit_on_selection: Vec<String>,
    pub toggle_fps: Vec<String>,
    pub toggle_fps_debug: Vec<String>,
    pub new_document: Vec<String>,

    // ── Parametric console-verb Actions ─────────────────────────
    // Each field is a list of `ParametricBinding`s. Args are
    // positional; per-variant shape is documented on the matching
    // `Action` variant. Defaults are empty — users opt in by adding
    // a binding to their `keybinds.json`.
    pub set_edge_anchor: Vec<ParametricBinding>,
    pub set_edge_body_glyph: Vec<ParametricBinding>,
    pub set_border_field: Vec<ParametricBinding>,
    pub set_edge_cap: Vec<ParametricBinding>,
    /// `args = [axis, value]` where `axis` is `bg|text|border`
    /// and `value` is a color string (`#rrggbb`, `var(--name)`,
    /// palette key). Maps to [`Action::SetColor`].
    pub set_color: Vec<ParametricBinding>,
    /// `args = [target_kind, field, value]` where `target_kind ∈
    /// {node, section, canvas-border, canvas-sf, canvas-sf-focused}`
    /// (kebab-case, matching [`crate::application::keybinds::BorderPreviewTargetKind`]'s
    /// strum-`IntoStaticStr`-derived form). `field` is one of the
    /// `border` kv keys (`preset`, `font`, `size`, `color`,
    /// `palette`, `field`, `padding`, `top`, `bottom`, `left`,
    /// `right`, `tl`, `tr`, `bl`, `br`); `value` is the same
    /// kv-form string the verb accepts. Maps to
    /// [`Action::SetBorderPreview`]. Pre-fix this Action variant
    /// existed but was *unregistered* — users could not bind a
    /// key to preview-set via JSON.
    pub set_border_preview: Vec<ParametricBinding>,
    pub set_edge_type: Vec<ParametricBinding>,
    pub set_edge_display_mode: Vec<ParametricBinding>,
    pub reset_edge: Vec<ParametricBinding>,
    pub set_font_family: Vec<ParametricBinding>,
    /// `args = [slot, pt]` where `slot` is `size|min|max` and `pt`
    /// is a positive finite float string. Maps to [`Action::SetFont`].
    pub set_font: Vec<ParametricBinding>,
    pub set_edge_label_text: Vec<ParametricBinding>,
    pub set_edge_label_position: Vec<ParametricBinding>,
    pub set_spacing: Vec<ParametricBinding>,
    /// `args = [bound, value]` where `bound` is `min|max` and
    /// `value` is `"unset"`, `""`, or a positive finite float
    /// string. Maps to [`Action::SetZoom`].
    pub set_zoom: Vec<ParametricBinding>,
    /// Unit variant — `args` is ignored, only `combo` matters.
    pub clear_zoom: Vec<ParametricBinding>,
    /// `args = [dx, dy]` parsed as f64. Maps to
    /// [`Action::SetSectionOffsetDelta`].
    pub set_section_offset_delta: Vec<ParametricBinding>,
    /// `args = [w, h]` parsed as f64. Maps to
    /// [`Action::SetSectionSizeAbs`].
    pub set_section_size_abs: Vec<ParametricBinding>,
    /// Unit variant — `args` ignored. Maps to
    /// [`Action::SetSectionSizeFillParent`].
    pub set_section_size_fill_parent: Vec<ParametricBinding>,
    /// Filesystem-touching parametric variants — NativeOnly +
    /// denylisted from non-User macro tiers (see
    /// `SourceTier::allows_action`).
    pub open_document: Vec<ParametricBinding>,
    pub save_document_as: Vec<ParametricBinding>,
    pub new_document_at: Vec<ParametricBinding>,

    // ── Style / metadata ─────────────────────────────────────────
    /// Font family name for the console overlay.
    pub console_font: String,
    /// Font size in pixels for the console overlay.
    pub console_font_size: f32,
    /// Map of key combo → custom mutation id.
    pub custom_mutation_bindings: HashMap<String, String>,
    /// Map of key combo → macro id. Macros are resolved AFTER built-in
    /// `Action`s and BEFORE custom mutations, so a key bound to both a
    /// macro and a custom mutation fires the macro.
    pub macro_bindings: HashMap<String, String>,
}

impl Default for KeybindConfig {
    fn default() -> Self {
        KeybindConfig {
            // Document-level
            undo: vec!["Ctrl+Z".into(), "Undo".into()],
            enter_reparent_mode: vec!["Ctrl+P".into()],
            enter_connect_mode: vec!["Ctrl+D".into()],
            enter_resize_mode: vec!["r".into(), "LongPress".into()],
            fast_resize_start: vec!["Ctrl+RightDrag".into(), "TwoFingerDrag".into()],
            delete_selection: vec!["Delete".into()],
            exit_mode: vec!["Escape".into()],
            create_orphan_node: vec!["Ctrl+N".into()],
            orphan_selection: vec!["Ctrl+O".into()],
            edit_selection: vec!["Enter".into()],
            edit_selection_clean: vec!["Backspace".into()],
            // `enter_node_edit` is reachable via the umbrella
            // `Enter` (EditSelection) keybind for node selections;
            // an explicit binding is offered for users who want
            // the no-fan-out version (e.g. macros).
            enter_node_edit: vec![],
            enter_node_edit_clean: vec![],
            // `Enter` at the NodeEdit context — same key the
            // umbrella uses at Document, but the cascade picks
            // NodeEdit when active.
            enter_section_edit: vec!["Enter".into()],
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
            console_scroll_up: vec!["Shift+ArrowUp".into(), "Shift+Up".into()],
            console_scroll_down: vec!["Shift+ArrowDown".into(), "Shift+Down".into()],
            console_scroll_page_up: vec!["PageUp".into()],
            console_scroll_page_down: vec!["PageDown".into()],
            console_scroll_end: vec!["Shift+End".into()],
            console_scroll_home: vec!["Shift+Home".into()],

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

            // Border Preview
            //
            // No default keybinding for `cancel` — Esc-cancels-preview
            // is achieved by chaining inside `ExitMode`'s arm body
            // (it calls `cancel_border_preview()` before mode-clear
            // and short-circuits when a preview was canceled).
            // Binding `Escape` here would shadow `exit_mode` because
            // the resolver picks the first match per `(context, key)`
            // and has no per-action active-state guard. Users who
            // want `cancel` on a different key opt in via JSON.
            // `commit_border_preview` is purely opt-in — there is
            // no implicit chain for it.
            cancel_border_preview: vec![],
            commit_border_preview: vec![],

            // Mouse-gesture Actions. `create_orphan_node_and_edit`
            // is intentionally `vec![]` —
            // the user found the empty-canvas double-click annoying;
            // it's now opt-in.
            double_click_activate: vec!["DoubleClick".into()],
            create_orphan_node_and_edit: vec![],
            pan_canvas: vec!["LeftDrag".into(), "MiddleClick".into()],

            // Navigation / camera. `ZoomIn`/`ZoomOut` default to mouse
            // wheel per user request; key shortcuts (e.g. Ctrl++/Ctrl+-)
            // can be added in user keybinds.json.
            zoom_in: vec!["WheelUp".into()],
            zoom_out: vec!["WheelDown".into()],
            zoom_reset: vec!["Ctrl+0".into()],
            zoom_fit: vec!["Ctrl+1".into()],
            pan_camera_north: vec!["Alt+ArrowUp".into()],
            pan_camera_south: vec!["Alt+ArrowDown".into()],
            pan_camera_east: vec!["Alt+ArrowRight".into()],
            pan_camera_west: vec!["Alt+ArrowLeft".into()],
            center_on_selection: vec!["Ctrl+.".into()],
            // Default unbound — `Home` is consumed by the text editor
            // when it's open (already routed to TextEditCursorHome),
            // and binding it at the Document level would shadow that
            // for users who haven't customized. Users who want a
            // jump-to-root key bind it themselves.
            jump_to_root: vec![],

            // Selection.
            select_all: vec!["Ctrl+a".into()],
            // Default unbound — Esc already exits modes via
            // `ExitMode`, and rebinding Esc here would conflict
            // with the modal-cascade contract. Users opt in by
            // binding e.g. `Ctrl+Shift+a`.
            deselect_all: vec![],
            invert_selection: vec!["Ctrl+Shift+i".into()],
            select_parent: vec!["Alt+p".into()],
            select_child: vec!["Alt+c".into()],
            select_next_sibling: vec!["Alt+j".into()],
            select_prev_sibling: vec!["Alt+k".into()],

            // TextEdit cursor primitives. Bodies live in
            // dispatch::apply_text_edit_action; the editor's modal
            // handler routes through dispatch when a binding matches
            // and falls back to literal-character insertion otherwise.
            text_edit_cursor_left: vec!["ArrowLeft".into()],
            text_edit_cursor_right: vec!["ArrowRight".into()],
            text_edit_cursor_up: vec!["ArrowUp".into()],
            text_edit_cursor_down: vec!["ArrowDown".into()],
            text_edit_cursor_home: vec!["Home".into()],
            text_edit_cursor_end: vec!["End".into()],
            text_edit_cursor_left_select: vec!["Shift+ArrowLeft".into()],
            text_edit_cursor_right_select: vec!["Shift+ArrowRight".into()],
            text_edit_cursor_up_select: vec!["Shift+ArrowUp".into()],
            text_edit_cursor_down_select: vec!["Shift+ArrowDown".into()],
            text_edit_cursor_home_select: vec!["Shift+Home".into()],
            text_edit_cursor_end_select: vec!["Shift+End".into()],
            text_edit_word_left: vec!["Ctrl+ArrowLeft".into()],
            text_edit_word_right: vec!["Ctrl+ArrowRight".into()],
            text_edit_delete_back: vec!["Backspace".into()],
            text_edit_delete_forward: vec!["Delete".into()],
            text_edit_delete_word_back: vec!["Ctrl+Backspace".into()],
            text_edit_delete_word_forward: vec!["Ctrl+Delete".into()],
            // Default unbound — Enter is literal `\n` in the multi-
            // line node editor. Users who want commit-on-Enter bind
            // it themselves (and lose newline insertion in exchange).
            // Note: any TextEdit Action bound to Enter (not just
            // Commit) wins over literal-newline insertion — the
            // action lookup runs before the literal-character
            // fallback in `handle_text_edit_key`. So binding
            // `text_edit_cursor_down: ["Enter"]` to a multi-line
            // editor would break newline insertion.
            text_edit_commit: vec![],

            // LabelEdit cursor primitives. Same routing path as
            // TextEdit but single-line — no Up/Down/Word*. Defaults
            // mirror what `route_single_line_key` previously
            // hardcoded.
            label_edit_cursor_left: vec!["ArrowLeft".into()],
            label_edit_cursor_right: vec!["ArrowRight".into()],
            label_edit_cursor_home: vec!["Home".into()],
            label_edit_cursor_end: vec!["End".into()],
            label_edit_delete_back: vec!["Backspace".into()],
            label_edit_delete_forward: vec!["Delete".into()],

            // Console-verb Actions. Defaults empty — these mirror
            // typed console verbs and the user opts in by binding
            // a key.
            open_color_picker: vec![],
            close_color_picker: vec![],
            label_edit_on_selection: vec![],
            toggle_fps: vec![],
            toggle_fps_debug: vec![],
            new_document: vec![],

            // Parametric console-verb Actions. Defaults empty — users
            // opt in via `keybinds.json` because there's no universal
            // sensible default for `from=top to=bottom`-style payloads.
            set_edge_anchor: vec![],
            set_edge_body_glyph: vec![],
            set_border_field: vec![],
            set_edge_cap: vec![],
            set_color: vec![],
            set_border_preview: vec![],
            set_edge_type: vec![],
            set_edge_display_mode: vec![],
            reset_edge: vec![],
            set_font_family: vec![],
            set_font: vec![],
            set_edge_label_text: vec![],
            set_edge_label_position: vec![],
            set_spacing: vec![],
            set_zoom: vec![],
            clear_zoom: vec![],
            set_section_offset_delta: vec![],
            set_section_size_abs: vec![],
            set_section_size_fill_parent: vec![],
            open_document: vec![],
            save_document_as: vec![],
            new_document_at: vec![],

            // Style / metadata
            console_font: String::new(),
            console_font_size: 16.0,
            custom_mutation_bindings: HashMap::new(),
            macro_bindings: HashMap::new(),
        }
    }
}

impl KeybindConfig {
    /// Parse a JSON string into a config. Missing fields fall back to
    /// defaults thanks to `#[serde(default)]` on the struct.
    pub fn from_json(json: &str) -> Result<Self, String> {
        baumhard::format::json::parse(json).map_err(|e| format!("parse keybinds JSON: {}", e))
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
            (Action::EnterResizeMode, &self.enter_resize_mode),
            (Action::FastResizeStart, &self.fast_resize_start),
            (Action::DeleteSelection, &self.delete_selection),
            (Action::ExitMode, &self.exit_mode),
            (Action::CreateOrphanNode, &self.create_orphan_node),
            (Action::OrphanSelection, &self.orphan_selection),
            (Action::EditSelection, &self.edit_selection),
            (Action::EditSelectionClean, &self.edit_selection_clean),
            (Action::EnterNodeEdit, &self.enter_node_edit),
            (Action::EnterNodeEditClean, &self.enter_node_edit_clean),
            (Action::EnterSectionEdit, &self.enter_section_edit),
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
            (Action::ConsoleScrollUp, &self.console_scroll_up),
            (Action::ConsoleScrollDown, &self.console_scroll_down),
            (Action::ConsoleScrollPageUp, &self.console_scroll_page_up),
            (Action::ConsoleScrollPageDown, &self.console_scroll_page_down),
            (Action::ConsoleScrollEnd, &self.console_scroll_end),
            (Action::ConsoleScrollHome, &self.console_scroll_home),
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
            // Border Preview
            (Action::CancelBorderPreview, &self.cancel_border_preview),
            (Action::CommitBorderPreview, &self.commit_border_preview),
            // Mouse-gesture Actions
            (Action::DoubleClickActivate, &self.double_click_activate),
            (Action::CreateOrphanNodeAndEdit, &self.create_orphan_node_and_edit),
            (Action::PanCanvas, &self.pan_canvas),
            // Navigation / camera
            (Action::ZoomIn, &self.zoom_in),
            (Action::ZoomOut, &self.zoom_out),
            (Action::ZoomReset, &self.zoom_reset),
            (Action::ZoomFit, &self.zoom_fit),
            (Action::PanCameraNorth, &self.pan_camera_north),
            (Action::PanCameraSouth, &self.pan_camera_south),
            (Action::PanCameraEast, &self.pan_camera_east),
            (Action::PanCameraWest, &self.pan_camera_west),
            (Action::CenterOnSelection, &self.center_on_selection),
            (Action::JumpToRoot, &self.jump_to_root),
            // Selection
            (Action::SelectAll, &self.select_all),
            (Action::DeselectAll, &self.deselect_all),
            (Action::InvertSelection, &self.invert_selection),
            (Action::SelectParent, &self.select_parent),
            (Action::SelectChild, &self.select_child),
            (Action::SelectNextSibling, &self.select_next_sibling),
            (Action::SelectPrevSibling, &self.select_prev_sibling),
            // TextEdit cursor primitives
            (Action::TextEditCursorLeft, &self.text_edit_cursor_left),
            (Action::TextEditCursorRight, &self.text_edit_cursor_right),
            (Action::TextEditCursorUp, &self.text_edit_cursor_up),
            (Action::TextEditCursorDown, &self.text_edit_cursor_down),
            (Action::TextEditCursorHome, &self.text_edit_cursor_home),
            (Action::TextEditCursorEnd, &self.text_edit_cursor_end),
            (
                Action::TextEditCursorLeftSelect,
                &self.text_edit_cursor_left_select,
            ),
            (
                Action::TextEditCursorRightSelect,
                &self.text_edit_cursor_right_select,
            ),
            (Action::TextEditCursorUpSelect, &self.text_edit_cursor_up_select),
            (
                Action::TextEditCursorDownSelect,
                &self.text_edit_cursor_down_select,
            ),
            (
                Action::TextEditCursorHomeSelect,
                &self.text_edit_cursor_home_select,
            ),
            (Action::TextEditCursorEndSelect, &self.text_edit_cursor_end_select),
            (Action::TextEditWordLeft, &self.text_edit_word_left),
            (Action::TextEditWordRight, &self.text_edit_word_right),
            (Action::TextEditDeleteBack, &self.text_edit_delete_back),
            (Action::TextEditDeleteForward, &self.text_edit_delete_forward),
            (Action::TextEditDeleteWordBack, &self.text_edit_delete_word_back),
            (
                Action::TextEditDeleteWordForward,
                &self.text_edit_delete_word_forward,
            ),
            (Action::TextEditCommit, &self.text_edit_commit),
            // LabelEdit cursor primitives
            (Action::LabelEditCursorLeft, &self.label_edit_cursor_left),
            (Action::LabelEditCursorRight, &self.label_edit_cursor_right),
            (Action::LabelEditCursorHome, &self.label_edit_cursor_home),
            (Action::LabelEditCursorEnd, &self.label_edit_cursor_end),
            (Action::LabelEditDeleteBack, &self.label_edit_delete_back),
            (Action::LabelEditDeleteForward, &self.label_edit_delete_forward),
            // Console-verb Actions
            (Action::OpenColorPicker, &self.open_color_picker),
            (Action::CloseColorPicker, &self.close_color_picker),
            (Action::LabelEditOnSelection, &self.label_edit_on_selection),
            (Action::ToggleFps, &self.toggle_fps),
            (Action::ToggleFpsDebug, &self.toggle_fps_debug),
            (Action::NewDocument, &self.new_document),
        ];
        for (action, strings) in sets {
            for s in *strings {
                match KeyBind::parse(s) {
                    Ok(k) => binds.push((action.clone(), k)),
                    Err(e) => warn!("keybinds: skipping invalid keybind '{}': {}", s, e),
                }
            }
        }

        // Parametric bindings: each variant carries its own payload
        // shape, so each `push_parametric` call passes a builder
        // closure that picks the args apart and constructs the
        // `Action`. Wrong arg counts emit a warn-log and are skipped
        // — never panic on a user-config typo.
        push_parametric(
            &mut binds,
            "set_edge_anchor",
            2,
            &self.set_edge_anchor,
            |args| match args {
                [from, to] => Some(Action::SetEdgeAnchor {
                    from: from.clone(),
                    to: to.clone(),
                }),
                _ => None,
            },
        );
        push_parametric(
            &mut binds,
            "set_edge_body_glyph",
            1,
            &self.set_edge_body_glyph,
            |args| match args {
                [glyph] => Some(Action::SetEdgeBodyGlyph(glyph.clone())),
                _ => None,
            },
        );
        push_parametric(
            &mut binds,
            "set_border_field",
            2,
            &self.set_border_field,
            |args| match args {
                [field, value] => Some(Action::SetBorderField {
                    field: field.clone(),
                    value: value.clone(),
                }),
                _ => None,
            },
        );
        push_parametric(
            &mut binds,
            "set_edge_cap",
            2,
            &self.set_edge_cap,
            |args| match args {
                [from, to] => Some(Action::SetEdgeCap {
                    from: from.clone(),
                    to: to.clone(),
                }),
                _ => None,
            },
        );
        // Color: `args = [axis, value]` where axis = `bg|text|border`.
        // `axis.parse::<ColorAxis>()` is the strum-`EnumString`-derived
        // round-trip with `IntoStaticStr` (`Bg.into() == "bg"` etc.) —
        // unrecognized tokens land in `Err`, which `push_parametric`
        // then warns and skips on.
        push_parametric(&mut binds, "set_color", 2, &self.set_color, |args| match args {
            [axis, value] => axis
                .parse::<super::ColorAxis>()
                .ok()
                .map(|axis| Action::SetColor {
                    axis,
                    value: value.clone(),
                }),
            _ => None,
        });
        // BorderPreview: `args = [target_kind, field, value]`.
        // `target_kind.parse::<BorderPreviewTargetKind>()` is the
        // strum-`EnumString`-derived round-trip; unknown tokens
        // land in `Err` and `push_parametric` warns and skips.
        push_parametric(
            &mut binds,
            "set_border_preview",
            3,
            &self.set_border_preview,
            |args| match args {
                [target_kind, field, value] => target_kind
                    .parse::<super::BorderPreviewTargetKind>()
                    .ok()
                    .map(|target_kind| Action::SetBorderPreview {
                        target_kind,
                        field: field.clone(),
                        value: value.clone(),
                    }),
                _ => None,
            },
        );
        // Edge structural — type / display_mode / reset.
        push_parametric(
            &mut binds,
            "set_edge_type",
            1,
            &self.set_edge_type,
            |args| match args {
                [t] => Some(Action::SetEdgeType(t.clone())),
                _ => None,
            },
        );
        push_parametric(
            &mut binds,
            "set_edge_display_mode",
            1,
            &self.set_edge_display_mode,
            |args| match args {
                [m] => Some(Action::SetEdgeDisplayMode(m.clone())),
                _ => None,
            },
        );
        push_parametric(&mut binds, "reset_edge", 1, &self.reset_edge, |args| match args {
            [kind] => Some(Action::ResetEdge(kind.clone())),
            _ => None,
        });
        // Font family / size / clamps + label + spacing.
        push_parametric(
            &mut binds,
            "set_font_family",
            1,
            &self.set_font_family,
            |args| match args {
                [family] => Some(Action::SetFontFamily(family.clone())),
                _ => None,
            },
        );
        // Font: `args = [slot, pt]` where slot = `size|min|max`.
        push_parametric(&mut binds, "set_font", 2, &self.set_font, |args| match args {
            [slot, value] => slot.parse::<super::FontSlot>().ok().map(|slot| Action::SetFont {
                slot,
                value: value.clone(),
            }),
            _ => None,
        });
        push_parametric(
            &mut binds,
            "set_edge_label_text",
            1,
            &self.set_edge_label_text,
            |args| match args {
                [text] => Some(Action::SetEdgeLabelText(text.clone())),
                _ => None,
            },
        );
        push_parametric(
            &mut binds,
            "set_edge_label_position",
            1,
            &self.set_edge_label_position,
            |args| match args {
                [pos] => Some(Action::SetEdgeLabelPosition(pos.clone())),
                _ => None,
            },
        );
        push_parametric(
            &mut binds,
            "set_spacing",
            1,
            &self.set_spacing,
            |args| match args {
                [v] => Some(Action::SetSpacing(v.clone())),
                _ => None,
            },
        );
        // Zoom: `args = [bound, value]` where bound = `min|max`.
        // `clear_zoom` is a separate unit variant.
        push_parametric(&mut binds, "set_zoom", 2, &self.set_zoom, |args| match args {
            [bound, value] => bound
                .parse::<super::ZoomBound>()
                .ok()
                .map(|bound| Action::SetZoom {
                    bound,
                    value: value.clone(),
                }),
            _ => None,
        });
        push_parametric(&mut binds, "clear_zoom", 0, &self.clear_zoom, |args| match args {
            [] => Some(Action::ClearZoom),
            _ => None,
        });
        push_parametric(
            &mut binds,
            "set_section_offset_delta",
            2,
            &self.set_section_offset_delta,
            |args| match args {
                [dx, dy] => Some(Action::SetSectionOffsetDelta {
                    dx: dx.clone(),
                    dy: dy.clone(),
                }),
                _ => None,
            },
        );
        push_parametric(
            &mut binds,
            "set_section_size_abs",
            2,
            &self.set_section_size_abs,
            |args| match args {
                [w, h] => Some(Action::SetSectionSizeAbs {
                    w: w.clone(),
                    h: h.clone(),
                }),
                _ => None,
            },
        );
        push_parametric(
            &mut binds,
            "set_section_size_fill_parent",
            0,
            &self.set_section_size_fill_parent,
            |args| match args {
                [] => Some(Action::SetSectionSizeFillParent),
                _ => None,
            },
        );
        // Filesystem variants — NativeOnly + privilege-gated.
        push_parametric(
            &mut binds,
            "open_document",
            1,
            &self.open_document,
            |args| match args {
                [path] => Some(Action::OpenDocument(path.clone())),
                _ => None,
            },
        );
        push_parametric(
            &mut binds,
            "save_document_as",
            1,
            &self.save_document_as,
            |args| match args {
                [path] => Some(Action::SaveDocumentAs(path.clone())),
                _ => None,
            },
        );
        push_parametric(
            &mut binds,
            "new_document_at",
            1,
            &self.new_document_at,
            |args| match args {
                [path] => Some(Action::NewDocumentAt(path.clone())),
                _ => None,
            },
        );

        let mut custom_binds: Vec<(KeyBind, String)> = Vec::new();
        for (combo, mutation_id) in &self.custom_mutation_bindings {
            match KeyBind::parse(combo) {
                Ok(k) => custom_binds.push((k, mutation_id.clone())),
                Err(e) => warn!(
                    "keybinds: skipping invalid custom_mutation_binding '{}': {}",
                    combo, e
                ),
            }
        }
        let mut macro_binds: Vec<(KeyBind, String)> = Vec::new();
        for (combo, macro_id) in &self.macro_bindings {
            match KeyBind::parse(combo) {
                Ok(k) => macro_binds.push((k, macro_id.clone())),
                Err(e) => warn!("keybinds: skipping invalid macro_binding '{}': {}", combo, e),
            }
        }

        ResolvedKeybinds::new(
            binds,
            custom_binds,
            macro_binds,
            self.console_font.clone(),
            self.console_font_size.max(4.0),
        )
    }
}

/// Resolve every binding for one parametric variant. The builder
/// closure picks the `Action` apart from the positional args; a
/// `None` return means "wrong arg count for this variant" — the
/// binding is logged and skipped (never panic on a user-config typo).
///
/// `expected_arity` is the count the builder closure expects (used
/// only to make the warn-log self-explanatory: a user typo'ing a
/// binding sees the verb name AND the arg count their config should
/// have used). Passing the right value here is mechanical — it has
/// to match the closure's accepted arm; mismatches just produce a
/// slightly less helpful warn message.
fn push_parametric<F>(
    binds: &mut Vec<(Action, KeyBind)>,
    name: &str,
    expected_arity: usize,
    bindings: &[ParametricBinding],
    build: F,
) where
    F: Fn(&[String]) -> Option<Action>,
{
    for binding in bindings {
        match KeyBind::parse(&binding.combo) {
            Ok(k) => match build(&binding.args) {
                Some(action) => binds.push((action, k)),
                None => warn!(
                    "keybinds: skipping {} binding '{}': wrong args (got {}, expected {})",
                    name,
                    binding.combo,
                    binding.args.len(),
                    expected_arity,
                ),
            },
            Err(e) => warn!("keybinds: skipping invalid keybind '{}': {}", binding.combo, e),
        }
    }
}
