// SPDX-License-Identifier: MPL-2.0

//! `KeybindConfig` — the user-editable config surface, its JSON
//! loader, and the `resolve()` step that produces the runtime
//! `ResolvedKeybinds` table.
//!
//! **The struct is generated.** Everything a bindable
//! [`Action`] needs on the config side — the struct field, its
//! default, its row in the resolve step, and its arm in the
//! `ActionKind` → JSON-key match — comes from the one
//! [`keybind_surface!`] table below. That is the whole point of the
//! module: the three used to be hand-synced, a field missing from
//! `resolve()` compiled, and the binding was silently dead. See
//! [`super::surface`] for the machinery and for what a new variant
//! now costs (a compile error until it appears in the table).
//!
//! What stays hand-written here is what is not per-Action: the JSON
//! entry point, the unrecognized-key report, and the tail of
//! `resolve()` that folds in the id-keyed custom-mutation and macro
//! binding maps.

use std::collections::{BTreeMap, HashMap};

use log::warn;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;

use super::action::{Action, ActionKind};
use super::bind::KeyBind;
use super::resolved::ResolvedKeybinds;
use super::surface::{keybind_surface, BindSurface, ParametricBinding};

keybind_surface! {
    // ─────────────────────────────────────────────────────────────
    // Unit Actions — a plain list of key combos per action.
    // ─────────────────────────────────────────────────────────────
    simple {
        // ── Document-level (global) ──────────────────────────────
        /// Undo the last document mutation.
        Undo => undo = ["Ctrl+Z", "Undo"],
        /// Enter reparent mode on the current selection.
        EnterReparentMode => enter_reparent_mode = ["Ctrl+P"],
        /// Enter connect mode on the current selection.
        EnterConnectMode => enter_connect_mode = ["Ctrl+D"],
        /// Enter resize mode on the current selection — Batch 2 of
        /// `SECTIONS_BORDERS_RESIZE_PLAN.md`.
        EnterResizeMode => enter_resize_mode = ["r", "LongPress"],
        /// Start a corner-anchored fast-resize gesture from a
        /// right-button drag. Lookup goes through
        /// [`super::ResolvedKeybinds::action_for_gesture`] so the
        /// standard modifier-fallback applies — a user who clears
        /// the default and binds bare `["RightDrag"]` gets
        /// fast-resize on plain right-drag, no Ctrl required.
        FastResizeStart => fast_resize_start = ["Ctrl+RightDrag"],
        /// Delete the current selection.
        DeleteSelection => delete_selection = ["Delete"],
        /// Leave the active interaction mode. Also cancels an active
        /// border preview first — see [`Action::ExitMode`].
        ExitMode => exit_mode = ["Escape"],
        /// Create an unattached node at the cursor.
        CreateOrphanNode => create_orphan_node = ["Ctrl+N"],
        /// Detach every selected node from its parent.
        OrphanSelection => orphan_selection = ["Ctrl+O"],
        /// The umbrella "edit the selection" action.
        EditSelection => edit_selection = ["Enter"],
        /// As `edit_selection`, but the editor opens empty.
        EditSelectionClean => edit_selection_clean = ["Backspace"],
        /// Enter NodeEdit mode (Batch 3 of
        /// `SECTIONS_BORDERS_RESIZE_PLAN.md`). The umbrella `Enter`
        /// keybind dispatches `edit_selection` first (which fans out
        /// here for node-bearing selections), so users normally
        /// don't rebind this — but the explicit hook is here for
        /// macros and future GUI buttons. Default unbound.
        EnterNodeEdit => enter_node_edit = [],
        /// As `enter_node_edit`, but the editor opens empty.
        EnterNodeEditClean => enter_node_edit_clean = [],
        /// Enter SectionEdit mode (text editor) on the active
        /// section while in NodeEdit mode. Default `Enter` at the
        /// [`super::InputContext::NodeEdit`] level — the same key as
        /// `edit_selection` at Document, since the contexts don't
        /// overlap (the modal cascade picks NodeEdit over Document
        /// when NodeEdit is active).
        EnterSectionEdit => enter_section_edit = ["Enter"],
        /// Open (or toggle) the console.
        OpenConsole => open_console = ["/"],
        /// Save the document to its bound path.
        SaveDocument => save_document = ["Ctrl+S"],
        /// Copy the focused component's clipboard representation.
        Copy => copy = ["Ctrl+C", "Copy"],
        /// Paste the clipboard into the focused component.
        Paste => paste = ["Ctrl+V", "Paste"],
        /// Cut: copy, then clear.
        Cut => cut = ["Ctrl+X", "Cut"],

        // ── Console ──────────────────────────────────────────────
        /// Close the console (dismiss the completion popup first).
        ConsoleClose => console_close = ["Escape"],
        /// Execute the current console line.
        ConsoleSubmit => console_submit = ["Enter"],
        /// Cycle tab completions.
        ConsoleTabComplete => console_tab_complete = ["Tab"],
        /// Walk history backward / move up the completion popup.
        ConsoleHistoryUp => console_history_up = ["ArrowUp", "Up"],
        /// Walk history forward / move down the completion popup.
        ConsoleHistoryDown => console_history_down = ["ArrowDown", "Down"],
        /// Move the console cursor one grapheme left.
        ConsoleCursorLeft => console_cursor_left = ["ArrowLeft", "Left"],
        /// Move the console cursor one grapheme right.
        ConsoleCursorRight => console_cursor_right = ["ArrowRight", "Right"],
        /// Move the console cursor to the start of the line.
        ConsoleCursorHome => console_cursor_home = ["Home"],
        /// Move the console cursor to the end of the line.
        ConsoleCursorEnd => console_cursor_end = ["End"],
        /// Delete the grapheme before the console cursor.
        ConsoleDeleteBack => console_delete_back = ["Backspace"],
        /// Delete the grapheme after the console cursor.
        ConsoleDeleteForward => console_delete_forward = ["Delete"],
        /// Insert a literal space (winit delivers Space as Named).
        ConsoleInsertSpace => console_insert_space = ["Space"],
        /// Clear the console input line (shell Ctrl+C).
        ConsoleClearLine => console_clear_line = ["Ctrl+C"],
        /// Jump to the start of the console line (shell Ctrl+A).
        ConsoleJumpStart => console_jump_start = ["Ctrl+A"],
        /// Jump to the end of the console line (shell Ctrl+E).
        ConsoleJumpEnd => console_jump_end = ["Ctrl+E"],
        /// Kill from the cursor to the start of the line (Ctrl+U).
        ConsoleKillToStart => console_kill_to_start = ["Ctrl+U"],
        /// Kill the word before the cursor (shell Ctrl+W).
        ConsoleKillWord => console_kill_word = ["Ctrl+W"],
        /// Scroll the scrollback up one line.
        ConsoleScrollUp => console_scroll_up = ["Shift+ArrowUp", "Shift+Up"],
        /// Scroll the scrollback down one line.
        ConsoleScrollDown => console_scroll_down = ["Shift+ArrowDown", "Shift+Down"],
        /// Scroll the scrollback up one page.
        ConsoleScrollPageUp => console_scroll_page_up = ["PageUp"],
        /// Scroll the scrollback down one page.
        ConsoleScrollPageDown => console_scroll_page_down = ["PageDown"],
        /// Pin the scrollback to the newest line.
        ConsoleScrollEnd => console_scroll_end = ["Shift+End"],
        /// Pin the scrollback to the oldest reachable line.
        ConsoleScrollHome => console_scroll_home = ["Shift+Home"],

        // ── Color Picker ─────────────────────────────────────────
        /// Cancel the color picker (contextual mode only).
        PickerCancel => picker_cancel = ["Escape"],
        /// Commit the picked color.
        PickerCommit => picker_commit = ["Enter"],
        /// Nudge hue down.
        PickerNudgeHueDown => picker_nudge_hue_down = ["h"],
        /// Nudge hue up.
        PickerNudgeHueUp => picker_nudge_hue_up = ["Shift+h"],
        /// Nudge saturation down.
        PickerNudgeSatDown => picker_nudge_sat_down = ["s"],
        /// Nudge saturation up.
        PickerNudgeSatUp => picker_nudge_sat_up = ["Shift+s"],
        /// Nudge value down.
        PickerNudgeValDown => picker_nudge_val_down = ["v"],
        /// Nudge value up.
        PickerNudgeValUp => picker_nudge_val_up = ["Shift+v"],

        // ── Label Editor ─────────────────────────────────────────
        /// Discard the inline label editor's changes.
        LabelEditCancel => label_edit_cancel = ["Escape"],
        /// Commit the inline label editor.
        LabelEditCommit => label_edit_commit = ["Enter"],

        // ── Text Editor ──────────────────────────────────────────
        /// Discard the inline text editor's changes.
        TextEditCancel => text_edit_cancel = ["Escape"],

        // ── Border / border preview ──────────────────────────────
        /// Discard the active border preview.
        ///
        /// **Default: unbound** — but the user-visible Esc behavior
        /// is "if a preview is active, cancel it; otherwise exit the
        /// current mode", because [`Action::ExitMode`] (the default
        /// Esc binding) calls `cancel_border_preview()` first and
        /// short-circuits if a preview was canceled. See
        /// `app/dispatch/action_core.rs` for the chain. The chain
        /// lives in `ExitMode`'s body rather than as a separate
        /// keybind because the resolver maps `(context, key) →
        /// Action` deterministically and can't fall through from one
        /// Action to another based on document state.
        ///
        /// Bind this entry to opt out of the chain — e.g. to put
        /// preview-cancel on a different key while leaving `Escape`
        /// bound to plain `ExitMode`. The verb path `border preview
        /// cancel` is the primary surface and works regardless.
        CancelBorderPreview => cancel_border_preview = [],
        /// Commit the active border preview through the matching
        /// committing setter. No default binding — users opt in.
        /// Unlike `cancel_border_preview`, commit isn't part of any
        /// implicit chain — the `border preview commit` verb is the
        /// primary surface and a binding here is purely opt-in
        /// muscle-memory.
        CommitBorderPreview => commit_border_preview = [],
        /// Advance the selected node(s)' border preset to the next
        /// entry in `BORDER_PRESETS`, wrapping at the end. Mirrors
        /// `border preset cycle`; default unbound.
        CycleBorderPreset => cycle_border_preset = [],
        /// Flip `style.show_frame` per selected node. Mirrors
        /// `border toggle`; default unbound.
        ToggleBorderVisible => toggle_border_visible = [],

        // ── Mouse-gesture Actions ────────────────────────────────
        /// Activate whatever the double-click hit — see
        /// [`Action::DoubleClickActivate`] for the four routes.
        DoubleClickActivate => double_click_activate = ["DoubleClick"],
        /// Create an orphan node and open its editor. Intentionally
        /// unbound by default: the user found the empty-canvas
        /// double-click annoying, so it is opt-in.
        CreateOrphanNodeAndEdit => create_orphan_node_and_edit = [],
        /// Continuous camera pan.
        PanCanvas => pan_canvas = ["LeftDrag", "MiddleClick"],

        // ── Navigation / camera ──────────────────────────────────
        // `zoom_in` / `zoom_out` default to the mouse wheel per user
        // request; key shortcuts (Ctrl++ / Ctrl+-) are opt-in.
        /// Zoom the camera in one step.
        ZoomIn => zoom_in = ["WheelUp"],
        /// Zoom the camera out one step.
        ZoomOut => zoom_out = ["WheelDown"],
        /// Reset the camera zoom to 1.0.
        ZoomReset => zoom_reset = ["Ctrl+0"],
        /// Fit the whole tree to the viewport.
        ZoomFit => zoom_fit = ["Ctrl+1"],
        /// Pan the camera north.
        PanCameraNorth => pan_camera_north = ["Alt+ArrowUp"],
        /// Pan the camera south.
        PanCameraSouth => pan_camera_south = ["Alt+ArrowDown"],
        /// Pan the camera east.
        PanCameraEast => pan_camera_east = ["Alt+ArrowRight"],
        /// Pan the camera west.
        PanCameraWest => pan_camera_west = ["Alt+ArrowLeft"],
        /// Center the camera on the selection's centroid.
        CenterOnSelection => center_on_selection = ["Ctrl+."],
        /// Jump camera + selection to the root node.
        ///
        /// Default unbound — `Home` is consumed by the text editor
        /// when it's open (already routed to
        /// `text_edit_cursor_home`), and binding it at the Document
        /// level would shadow that for users who haven't
        /// customized. Users who want a jump-to-root key bind it
        /// themselves.
        JumpToRoot => jump_to_root = [],

        // ── Selection ────────────────────────────────────────────
        /// Select every node in the document.
        SelectAll => select_all = ["Ctrl+a"],
        /// Clear the selection.
        ///
        /// Default unbound — Esc already exits modes via
        /// `exit_mode`, and rebinding Esc here would conflict with
        /// the modal-cascade contract. Users opt in by binding e.g.
        /// `Ctrl+Shift+a`.
        DeselectAll => deselect_all = [],
        /// Invert the selection.
        InvertSelection => invert_selection = ["Ctrl+Shift+i"],
        /// Select the parent of the selected node.
        SelectParent => select_parent = ["Alt+p"],
        /// Select the first child of the selected node.
        SelectChild => select_child = ["Alt+c"],
        /// Select the next sibling.
        SelectNextSibling => select_next_sibling = ["Alt+j"],
        /// Select the previous sibling.
        SelectPrevSibling => select_prev_sibling = ["Alt+k"],

        // ── TextEdit cursor primitives ───────────────────────────
        // Bodies live in `dispatch::apply_text_edit_action`; the
        // editor's modal handler routes through dispatch when a
        // binding matches and falls back to literal-character
        // insertion otherwise.
        /// Move the editor cursor one grapheme left.
        TextEditCursorLeft => text_edit_cursor_left = ["ArrowLeft"],
        /// Move the editor cursor one grapheme right.
        TextEditCursorRight => text_edit_cursor_right = ["ArrowRight"],
        /// Move the editor cursor one visual line up.
        TextEditCursorUp => text_edit_cursor_up = ["ArrowUp"],
        /// Move the editor cursor one visual line down.
        TextEditCursorDown => text_edit_cursor_down = ["ArrowDown"],
        /// Jump the editor cursor to the start of the line.
        TextEditCursorHome => text_edit_cursor_home = ["Home"],
        /// Jump the editor cursor to the end of the line.
        TextEditCursorEnd => text_edit_cursor_end = ["End"],
        /// Extend the selection one grapheme left.
        TextEditCursorLeftSelect => text_edit_cursor_left_select = ["Shift+ArrowLeft"],
        /// Extend the selection one grapheme right.
        TextEditCursorRightSelect => text_edit_cursor_right_select = ["Shift+ArrowRight"],
        /// Extend the selection one visual line up.
        TextEditCursorUpSelect => text_edit_cursor_up_select = ["Shift+ArrowUp"],
        /// Extend the selection one visual line down.
        TextEditCursorDownSelect => text_edit_cursor_down_select = ["Shift+ArrowDown"],
        /// Extend the selection to the start of the line.
        TextEditCursorHomeSelect => text_edit_cursor_home_select = ["Shift+Home"],
        /// Extend the selection to the end of the line.
        TextEditCursorEndSelect => text_edit_cursor_end_select = ["Shift+End"],
        /// Move the editor cursor one word left.
        TextEditWordLeft => text_edit_word_left = ["Ctrl+ArrowLeft"],
        /// Move the editor cursor one word right.
        TextEditWordRight => text_edit_word_right = ["Ctrl+ArrowRight"],
        /// Delete the grapheme before the editor cursor.
        TextEditDeleteBack => text_edit_delete_back = ["Backspace"],
        /// Delete the grapheme after the editor cursor.
        TextEditDeleteForward => text_edit_delete_forward = ["Delete"],
        /// Delete back through the current word.
        TextEditDeleteWordBack => text_edit_delete_word_back = ["Ctrl+Backspace"],
        /// Delete forward through the current word.
        TextEditDeleteWordForward => text_edit_delete_word_forward = ["Ctrl+Delete"],
        /// Commit the editor buffer and close.
        ///
        /// Default unbound — Enter is a literal `\n` in the
        /// multi-line node editor. Users who want commit-on-Enter
        /// bind it themselves (and lose newline insertion in
        /// exchange). Note: *any* TextEdit action bound to Enter
        /// wins over literal-newline insertion — the action lookup
        /// runs before the literal-character fallback in
        /// `handle_text_edit_key`.
        TextEditCommit => text_edit_commit = [],

        // ── LabelEdit cursor primitives ──────────────────────────
        // Same routing path as the TextEdit primitives but
        // single-line — no Up/Down/Word*. Defaults mirror what
        // `route_single_line_key` previously hardcoded.
        /// Move the label cursor one grapheme left.
        LabelEditCursorLeft => label_edit_cursor_left = ["ArrowLeft"],
        /// Move the label cursor one grapheme right.
        LabelEditCursorRight => label_edit_cursor_right = ["ArrowRight"],
        /// Jump the label cursor to the start of the buffer.
        LabelEditCursorHome => label_edit_cursor_home = ["Home"],
        /// Jump the label cursor to the end of the buffer.
        LabelEditCursorEnd => label_edit_cursor_end = ["End"],
        /// Delete the grapheme before the label cursor.
        LabelEditDeleteBack => label_edit_delete_back = ["Backspace"],
        /// Delete the grapheme after the label cursor.
        LabelEditDeleteForward => label_edit_delete_forward = ["Delete"],

        // ── Console-verb Actions ─────────────────────────────────
        // These mirror typed console verbs; the user opts in by
        // binding a key, so every default is empty.
        /// Open the glyph-wheel picker standalone (`color picker on`).
        OpenColorPicker => open_color_picker = [],
        /// Close the glyph-wheel picker (`color picker off`).
        CloseColorPicker => close_color_picker = [],
        /// Open the label editor on the selected edge (`label edit`).
        LabelEditOnSelection => label_edit_on_selection = [],
        /// Toggle the FPS overlay (`fps on` ↔ `fps off`).
        ToggleFps => toggle_fps = [],
        /// Toggle the FPS overlay's debug variant (`fps debug`).
        ToggleFpsDebug => toggle_fps_debug = [],
        /// Replace the document with a fresh blank one (`new`).
        NewDocument => new_document = [],
        /// Drop both zoom clamps on the selection (`zoom clear`).
        ClearZoom => clear_zoom = [],
        /// Flip the selected section back to fill-parent
        /// (`section resize fill`).
        SetSectionSizeFillParent => set_section_size_fill_parent = [],
        /// Delete the selected section (`section delete`). Errors
        /// when the node has only one section — the model invariant.
        DeleteSection => delete_section = [],
    }

    // ─────────────────────────────────────────────────────────────
    // Payload-carrying Actions — `{ "combo": …, "args": [ … ] }`.
    // The names in the payload pattern are the arg names quoted back
    // at the user when a binding's `args` array is the wrong length,
    // and they pick each field's `ArgValue` conversion.
    // ─────────────────────────────────────────────────────────────
    parametric {
        /// `anchor from=<side> to=<side>` on the selected edge.
        /// Sides: `auto|top|right|bottom|left`.
        SetEdgeAnchor { from, to } => set_edge_anchor,
        /// `body glyph=<dot|dash|double|wave|chain>`.
        SetEdgeBodyGlyph(glyph) => set_edge_body_glyph,
        /// `border <field>=<value>` on the selected node(s). One kv
        /// per binding; multi-kv border edits stay console-only.
        SetBorderField { field, value } => set_border_field,
        /// `cap from=<arrow|circle|diamond|none> to=<…>`.
        SetEdgeCap { from, to } => set_edge_cap,
        /// `args = [axis, value]` where `axis` is `bg|text|border`
        /// and `value` is a color string (`#rrggbb`, `var(--name)`,
        /// palette key). Maps to [`Action::SetColor`].
        SetColor { axis, value } => set_color,
        /// `args = [target_kind, field, value]` where `target_kind ∈
        /// {node, section, canvas-border, canvas-sf, canvas-sf-focused}`
        /// (kebab-case, matching [`super::BorderPreviewTargetKind`]'s
        /// strum-`IntoStaticStr`-derived form). `field` is one of the
        /// `border` kv keys (`preset`, `font`, `size`, `color`,
        /// `palette`, `field`, `padding`, `top`, `bottom`, `left`,
        /// `right`, `tl`, `tr`, `bl`, `br`); `value` is the same
        /// kv-form string the verb accepts. Maps to
        /// [`Action::SetBorderPreview`].
        SetBorderPreview { target_kind, field, value } => set_border_preview,
        /// `edge type=<cross_link|parent_child>`.
        SetEdgeType(edge_type) => set_edge_type,
        /// `edge display_mode=<line|portal>`.
        SetEdgeDisplayMode(display_mode) => set_edge_display_mode,
        /// `edge reset=<straight|curve|style|position>`.
        ResetEdge(reset_kind) => reset_edge,
        /// `font set <family>` on the current selection.
        SetFontFamily(family) => set_font_family,
        /// `args = [slot, pt]` where `slot` is `size|min|max` and
        /// `pt` is a positive finite float string. Maps to
        /// [`Action::SetFont`].
        SetFont { slot, value } => set_font,
        /// `label text=<text>`; an empty payload clears the label.
        SetEdgeLabelText(text) => set_edge_label_text,
        /// `label position=<start|middle|end>`.
        SetEdgeLabelPosition(position) => set_edge_label_position,
        /// `spacing value=<tight|normal|wide|<float>>`.
        SetSpacing(value) => set_spacing,
        /// `args = [bound, value]` where `bound` is `min|max` and
        /// `value` is `"unset"`, `""`, or a positive finite float
        /// string. Maps to [`Action::SetZoom`].
        SetZoom { bound, value } => set_zoom,
        /// `args = [dx, dy]` parsed as f64 at dispatch. Maps to
        /// [`Action::SetSectionOffsetDelta`].
        SetSectionOffsetDelta { dx, dy } => set_section_offset_delta,
        /// `args = [x, y]` parsed as f64 at dispatch — the absolute
        /// form of `set_section_offset_delta`. Maps to
        /// [`Action::SetSectionOffsetAbs`].
        SetSectionOffsetAbs { x, y } => set_section_offset_abs,
        /// `args = [w, h]` parsed as f64 at dispatch. Maps to
        /// [`Action::SetSectionSizeAbs`].
        SetSectionSizeAbs { w, h } => set_section_size_abs,
        /// `args = [text, runs_mode]` where `runs_mode` is
        /// `preserve` (clip existing runs to the new length) or
        /// `clear` (collapse to a single run). Maps to
        /// [`Action::SetSectionText`].
        SetSectionText { text, runs_mode } => set_section_text,
        /// `args = [at, text]` where `at` is `""` (append) or a
        /// section index, and `text` is the new section's body
        /// (`""` for an empty section). Maps to
        /// [`Action::AddSection`].
        AddSection { at, text } => add_section,
        /// `args = [at_grapheme]` — the grapheme offset to split the
        /// active section at. An empty string is rejected, matching
        /// the verb. Maps to [`Action::SplitSection`].
        SplitSection { at_grapheme } => split_section,
        /// `open <path>` — replace the document with the one at
        /// `path`. Filesystem-touching: NativeOnly and denylisted
        /// from non-User macro tiers (see
        /// `SourceTier::allows_action`).
        OpenDocument(path) => open_document,
        /// `save <path>` — write the document to `path` and rebind.
        /// Same NativeOnly + denylist posture as `open_document`.
        SaveDocumentAs(path) => save_document_as,
        /// `new <path>` — start a fresh document bound to `path`.
        /// Same NativeOnly + denylist posture as `open_document`.
        NewDocumentAt(path) => new_document_at,
    }

    // ─────────────────────────────────────────────────────────────
    // Actions with no key surface. The reason is required and is
    // published as documentation on `bind_surface`; the line it has
    // to be able to state is "a key could not supply this Action's
    // payload", not "nobody has wired it up yet".
    // ─────────────────────────────────────────────────────────────
    unbindable {
        ReparentToTarget => "confirmation dispatched by the click handler; the payload is the \
                             hit-tested target node id (`None` for empty canvas), which a static \
                             binding cannot supply",
        ConnectToTarget => "confirmation dispatched by the click handler; same hit-test payload as \
                            `ReparentToTarget`",
    }

    // ─────────────────────────────────────────────────────────────
    // Top-level keys that are not per-Action.
    // ─────────────────────────────────────────────────────────────
    extra {
        /// Font family name for the console overlay.
        pub console_font: String = String::new(),
        /// Font size in pixels for the console overlay. Clamped to a
        /// 4.0 floor at resolve time.
        pub console_font_size: f32 = 16.0,
        /// Map of key combo → custom mutation id.
        pub custom_mutation_bindings: HashMap<String, String> = HashMap::new(),
        /// Map of key combo → macro id. Macros resolve AFTER
        /// built-in `Action`s and BEFORE custom mutations, so a key
        /// bound to both a macro and a custom mutation fires the
        /// macro.
        pub macro_bindings: HashMap<String, String> = HashMap::new(),
    }
}

impl KeybindConfig {
    /// Parse a JSON string into a config. Missing fields fall back to
    /// defaults thanks to `#[serde(default)]` on the generated
    /// struct, and unrecognized top-level keys are warned about
    /// rather than rejected — see
    /// [`Self::unknown_top_level_keys`] for why.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let config: Self =
            baumhard::format::json::parse(json).map_err(|e| format!("parse keybinds JSON: {}", e))?;
        let unknown = Self::unknown_top_level_keys(json);
        if !unknown.is_empty() {
            // `known_keys()` allocates, so it is built once for the
            // whole report rather than per offending key.
            let known = Self::known_keys();
            for key in unknown {
                match nearest_known_key(&key, &known) {
                    Some(near) => warn!(
                        "keybinds: unrecognized key '{}' — ignored. Did you mean '{}'? Keys \
                         starting with '_' are treated as comments.",
                        key, near,
                    ),
                    None => warn!(
                        "keybinds: unrecognized key '{}' — ignored. None of the {} keys this \
                         config recognizes is close to it; keys starting with '_' are treated as \
                         comments.",
                        key,
                        known.len(),
                    ),
                }
            }
        }
        Ok(config)
    }

    /// The top-level keys in `json` that this config does not
    /// recognize, sorted. Keys starting with `_` are skipped: JSON
    /// has no comment syntax, and the shipped
    /// `config/default_keybinds.json` carries its instructions in a
    /// `_comment` key, so the underscore prefix is the documented
    /// way to annotate a config.
    ///
    /// **Warn, don't reject — for an unrecognized *key*.** A
    /// hand-edited config is exactly the surface where a
    /// silently-ignored key is the whole bug: a renamed action
    /// (`cancel_mode` → `exit_mode`) left the shipped template's Esc
    /// entry inert and said nothing. But `deny_unknown_fields` would
    /// answer one stale key by discarding every good binding beside
    /// it, so the loader keeps the config and names the key instead.
    ///
    /// **A malformed *value* under a recognized key is the opposite,
    /// and this is not the good outcome.** Serde fails the whole
    /// document, [`Self::from_json`] returns `Err`, and
    /// `user_config::layered::load_layered` answers a failed parse by
    /// logging one warning and walking to the next layer — so one
    /// bad value costs the user every good binding in the same file,
    /// which is precisely what the paragraph above rejects. The
    /// asymmetry is real, not a design: it falls out of parsing the
    /// document in one serde pass, and the same seam gives
    /// `macros.json` and `mutations.json` the same behavior. Issue
    /// #129 tracks the fix (per-field parsing with warn-and-skip, so
    /// a malformed value costs its own field and nothing else);
    /// `test_clear_zoom_rejects_the_parametric_binding_shape` pins
    /// the blast radius until then.
    ///
    /// Costs one extra pass over the source, keys only
    /// ([`IgnoredAny`] skips the values), on a config that is parsed
    /// once per launch. A source that is not a JSON object yields no
    /// keys — the typed parse ahead of it has already failed.
    pub fn unknown_top_level_keys(json: &str) -> Vec<String> {
        let Ok(raw) = baumhard::format::json::parse::<BTreeMap<String, IgnoredAny>>(json) else {
            return Vec::new();
        };
        let known = Self::known_keys();
        raw.into_keys()
            .filter(|key| !key.starts_with('_') && !known.contains(&key.as_str()))
            .collect()
    }

    /// Parse every binding string into concrete `KeyBind` values. Any
    /// binding that fails to parse is logged and skipped so a single
    /// typo doesn't break the entire config.
    ///
    /// The per-Action half is [`Self::push_action_binds`], generated
    /// from the `keybind_surface!` table; what is left here is the
    /// id-keyed maps, which are keyed by combo rather than by action
    /// and so have no table row.
    pub fn resolve(&self) -> ResolvedKeybinds {
        let mut binds: Vec<(Action, KeyBind)> = Vec::new();
        self.push_action_binds(&mut binds);

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

/// The entry of `known` closest to `key`, when one is close enough
/// to be worth quoting back at the user.
///
/// The unrecognized-key warning used to point at
/// `config/default_keybinds.json`, which lists nine of the keys the
/// schema has: a user who misspelled anything outside that nine was
/// sent to a file that would not contain the right spelling either.
/// [`KeybindConfig::known_keys`] is the actual schema, so the report
/// is built from it — a candidate when there is one, the recognized
/// count when there is not.
///
/// Distance is Levenshtein over **grapheme clusters**, not bytes or
/// `char`s: the key comes from a hand-edited file and may be
/// anything at all (CODE_CONVENTIONS §1). The acceptance threshold
/// is a third of the longer name, which admits `exit_modes` →
/// `exit_mode` and `set_colour` → `set_color` while refusing
/// `cancel_mode` → `exit_mode`; an invented key gets the count
/// rather than a misleading guess.
///
/// Ties go to the lexicographically first candidate so the message
/// is deterministic. Costs one `Vec<String>` per name plus an
/// `O(n·m)` two-row grid per candidate, over ~135 candidates, on a
/// config parsed once per launch.
pub(super) fn nearest_known_key(key: &str, known: &[&'static str]) -> Option<&'static str> {
    use baumhard::util::grapheme_chad::split_graphemes_owned;

    let target = split_graphemes_owned(key);
    let mut best: Option<(usize, &'static str)> = None;
    for candidate in known {
        let other = split_graphemes_owned(candidate);
        let longer = target.len().max(other.len());
        let distance = grapheme_levenshtein(&target, &other);
        if distance * 3 > longer {
            continue;
        }
        let better = match best {
            None => true,
            Some((best_distance, best_key)) => {
                distance < best_distance || (distance == best_distance && *candidate < best_key)
            }
        };
        if better {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// Levenshtein edit distance between two grapheme-cluster
/// sequences. Two rows rather than a full grid — the caller only
/// wants the number, and the longest key here is a few dozen
/// clusters, so the allocation is what dominates either way.
fn grapheme_levenshtein(a: &[String], b: &[String]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current: Vec<usize> = vec![0; b.len() + 1];
    for (i, ga) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, gb) in b.iter().enumerate() {
            let substitute = previous[j] + usize::from(ga != gb);
            current[j + 1] = substitute.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}
