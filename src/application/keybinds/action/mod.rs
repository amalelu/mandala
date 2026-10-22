// SPDX-License-Identifier: MPL-2.0

//! `Action` — the abstract user-action vocabulary the event loop
//! dispatches on. New keyboard interactions go here; the config side
//! is one row in the `keybind_surface!` table in
//! [`super::config`], and the build fails until you write it.

use serde::{Deserialize, Serialize};
use strum_macros::{EnumDiscriminants, EnumIter, EnumString, IntoStaticStr};

use super::context::InputContext;

// ── Action payload enums ─────────────────────────────────────────
//
// Typed discriminators for the parametric Action variants whose
// underlying console verb fans across a small fixed set of axes.
// Promoting these into the variant payload (instead of one Action
// variant per axis) keeps the dispatcher's match arm exhaustive
// without a `_ => log::error!(...)` "fan-out missed inner-match"
// guard, and eliminates the `set_color_bg / _text / _border`-style
// listing tax across `KeybindConfig`, `is_destructive`,
// `wasm_compatibility`, and `context`.
//
// Each carries a kv-key string at the boundary so the verb-core's
// existing `apply_kvs`-style trait dispatch reaches the same code
// path the typed `bg|text|border` console kv would have hit.

/// Which color axis a [`Action::SetColor`] targets. Mirrors the
/// `bg|text|border` kv key on the `color` console verb. The strum
/// derives generate the bidirectional kv-key conversion: `Bg.into()
/// == "bg"` (via `IntoStaticStr`) and `"bg".parse::<ColorAxis>() ==
/// Ok(Bg)` (via `EnumString`) — the verb-core consults these at the
/// boundary, replacing what was three hand-rolled match blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ColorAxis {
    Bg,
    Text,
    Border,
}

/// Which border-slot a [`Action::SetBorderPreview`] targets.
/// Discriminator for the `<verb> preview <kv>=…` Action surface
/// — five variants mirror the five committing setters
/// (`set_node_border_config` /
/// `set_section_frame_border_config` /
/// `set_canvas_default_border` /
/// `set_canvas_default_section_frame_border_config(focused=false|true)`).
///
/// Same strum-derive shape as [`ColorAxis`]: `Node.into() ==
/// "node"`, `"node".parse::<BorderPreviewTargetKind>() ==
/// Ok(Node)`. Replaces the prior `target_kind: String`
/// stringly-typed discriminator that could accept typos at
/// runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr, EnumString)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum BorderPreviewTargetKind {
    /// `border preview` — per-node `style.border` for selected nodes.
    Node,
    /// `section frame preview` — per-section `frame_border` for
    /// selected section pairs.
    Section,
    /// `canvas border preview` — `Canvas.default_border`.
    CanvasBorder,
    /// `canvas section-frame preview` — `Canvas.default_section_frame_border`.
    CanvasSf,
    /// `canvas section-frame focused preview` — `Canvas.default_focused_section_frame_border`.
    CanvasSfFocused,
}

/// Which font slot a [`Action::SetFont`] targets. Mirrors the
/// `size|min|max` kv key on the `font` console verb. Family
/// (`SetFontFamily`) lives on its own Action because the verb
/// dispatches it through a different code path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FontSlot {
    Size,
    Min,
    Max,
}

/// Which zoom bound a [`Action::SetZoom`] targets. Mirrors the
/// `min|max` kv key on the `zoom` console verb. Clearing both at
/// once lives on its own [`Action::ClearZoom`] variant — the
/// verb-core takes a `(min, max)` pair where each half is an
/// `OptionEdit<f32>`, and `Clear/Clear` is a meaningfully distinct
/// operation from `Keep/Keep`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ZoomBound {
    Min,
    Max,
}

/// High-level user actions that can be bound to keys. Add a new
/// variant here when a new keyboard interaction is introduced, give
/// it a row in the `keybind_surface!` table (which generates its
/// `KeybindConfig` field, its default and its resolve row from that
/// one row — or records why it has no key surface), and handle the
/// variant in the event loop. Skipping the table row is a compile
/// error, not a silently dead binding: see [`super::surface`].
///
/// `Serialize` / `Deserialize` are derived so macros can carry actions
/// in their JSON payload — see `crate::application::macros::MacroStep`.
///
/// `PayloadFieldNames` publishes each payload variant's field names,
/// in the order written below, into `action_payload_fields`. The
/// `keybind_surface!` table restates those names to define the
/// positional `args` contract of a `keybinds.json` binding, and the
/// two lists are compared at compile time — so the field order in
/// this declaration is load-bearing, and reordering a variant's
/// fields without reordering its table row is `error[E0080]` rather
/// than a user's arguments arriving in the wrong slots.
///
/// `#[non_exhaustive]` because new variants need to be reviewed
/// against the macro privilege gate
/// (`SourceTier::allows_action`) before they ship — the gate uses
/// a denylist of destructive Actions, and a new I/O / clipboard /
/// document-lifecycle variant added without updating the denylist
/// would silently bypass the gate from non-User macro tiers. The
/// `#[non_exhaustive]` is the structural signal that "review the
/// gate when extending."
/// Some variants carry payload (e.g. `String` paths, kv-shaped
/// `(field, value)` tuples) so `Copy` is impossible — payload-bearing
/// variants are cloned at lookup time in
/// [`super::resolved::ResolvedKeybinds::action_for_context`]. Each
/// keypress allocates one short string per `String`-payload variant
/// (two for `SetEdgeAnchor` / `SetEdgeCap` / `SetBorderField`). For
/// the typical interactive cadence (≤10 keypresses/sec) the cost is
/// inconsequential; for a synthetic load of macros firing thousands
/// of `Action`s/sec, switch the lookup to return `&Action` and clone
/// at the dispatch boundary, or wrap payload strings in `Arc<str>`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    EnumDiscriminants,
    mandala_derive::ActionClassify,
    mandala_derive::PayloadFieldNames,
)]
#[strum_discriminants(
    name(ActionKind),
    derive(Hash, EnumIter),
    doc = "Variant-kind mirror of [`Action`] without payloads. Generated by \
           strum's `EnumDiscriminants`. Used by the classifier methods \
           ([`is_destructive`](Self::is_destructive), \
           [`context`](Self::context), \
           [`wasm_compatibility`](Self::wasm_compatibility)) so the \
           classification matches need not destructure payloads, and by \
           tests that need to iterate every variant via `ActionKind::iter()`. \
           `&Action` and `Action` both `into()` an `ActionKind` (auto-derived)."
)]
#[non_exhaustive]
pub enum Action {
    // ── Document-level (global) ──────────────────────────────────
    /// Undo the last action on the document.
    #[action(context = Document, wasm = Compatible)]
    Undo,
    /// Enter reparent mode for the currently selected nodes.
    #[action(context = Document, wasm = NativeOnly)]
    EnterReparentMode,
    /// Enter connect mode for the currently selected node.
    #[action(context = Document, wasm = NativeOnly)]
    EnterConnectMode,
    /// Confirm a reparent operation by clicking on a target node
    /// (or empty canvas to promote sources to root). Sources come
    /// from `InteractionMode::Reparent { sources }`; the payload
    /// carries the target node id (`None` for empty-canvas → root).
    /// NativeOnly today because the click-handler path that surfaces
    /// the hit target lives natively only; the mode enum itself is
    /// cross-platform. Classified `is_destructive = true` so the
    /// privilege gate (`SourceTier::allows_action`) denylists non-User
    /// macro tiers; the arm body's `mem::replace(.., Default)` is
    /// an additional runtime guard (stale fire outside Reparent
    /// mode is a no-op).
    #[action(context = Document, wasm = NativeOnly, destructive)]
    ReparentToTarget(Option<String>),
    /// Confirm a connect operation by clicking on a target node
    /// (or empty canvas to exit Connect mode without creating an
    /// edge). Source comes from `InteractionMode::Connect { source }`; the
    /// payload carries the target node id (`None` for empty-canvas
    /// → mode-exit only, mirroring `ReparentToTarget`'s shape).
    /// NativeOnly + `is_destructive = true` per the same reasoning
    /// as `ReparentToTarget`.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    ConnectToTarget(Option<String>),
    /// Delete the current selection (currently: selected edge).
    #[action(context = Document, wasm = Compatible, destructive)]
    DeleteSelection,
    /// Exit the active interaction mode (Reparent / Connect / Resize)
    /// back to `Default`. Cross-platform: the mode-clear + scene
    /// rebuild slice runs on both targets via `dispatch_compatible`;
    /// the native-only residual (clearing `hovered_node` for the
    /// Reparent/Connect overlay rebuild) runs in the native arm.
    /// Replaces the pre-Batch-2 `CancelMode` per
    /// `SECTIONS_BORDERS_RESIZE_PLAN.md` §3.3 + CODE_CONVENTIONS §10
    /// (rename rather than alias).
    #[action(context = Document, wasm = Compatible)]
    ExitMode,
    /// Create a new unattached (orphan) node at the cursor position.
    #[action(context = Document, wasm = Compatible, destructive)]
    CreateOrphanNode,
    /// Detach every currently selected node from its parent.
    #[action(context = Document, wasm = Compatible, destructive)]
    OrphanSelection,
    /// Open the inline text editor on the currently selected single node
    /// with the node's existing text, cursor at end.
    ///
    /// **Today this is the umbrella "edit" Action** — for node /
    /// section / SectionRange selections it dispatches through
    /// [`Action::EnterNodeEdit`]; for `EdgeLabel` / `PortalLabel` /
    /// `PortalText` it opens the relevant inline editor directly.
    /// Multi / MultiSection / Edge / None silently no-op.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    EditSelection,
    /// Same as `EditSelection` but opens the editor with an empty buffer.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    EditSelectionClean,
    /// Enter NodeEdit mode on the currently selected node.
    /// Resolution rules:
    /// - `Single(node)` / `Section(s)` / `SectionRange { sel: s, .. }` →
    ///   `InteractionMode::NodeEdit { node_id }` (where `node_id` is
    ///   the owning node).
    /// - **Single-section short-circuit**: if the active node has
    ///   `sections.len() == 1`, the helper opens the text editor on
    ///   section 0 in the same call. This preserves today's
    ///   "Enter on a node opens the editor" UX for legacy migrated
    ///   maps. Multi-section nodes stay in NodeEdit and let the user
    ///   pick which section to edit (a second Enter, or a click on
    ///   a section followed by Enter).
    /// - `Multi` / `MultiSection` / `Edge*` / `None` → no-op + log.
    ///
    /// **WASM: NativeOnly** — opening the text editor depends on
    /// `TextEditState`, which is part of the native modal-stealer
    /// cascade. The `apply_enter_node_edit` helper itself is
    /// cross-platform-shaped; reclassification is a one-line change
    /// once WASM gains the modal pipeline.
    ///
    /// `destructive` — the single-section short-circuit opens the
    /// editor, which can clear text on the spot. User-tier-only via
    /// the macro privilege gate.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    EnterNodeEdit,
    /// Same as [`Action::EnterNodeEdit`] but the (potentially-opened)
    /// editor starts with an empty buffer rather than the section's
    /// existing text. Mirrors the `EditSelectionClean` posture.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    EnterNodeEditClean,
    /// Open the section text editor on the active section while in
    /// NodeEdit mode.
    /// - Active mode must be `InteractionMode::NodeEdit { node_id }`.
    /// - Selection determines the section: `Section(s)` /
    ///   `SectionRange { sel: s, .. }` use `s.section_idx`;
    ///   `Single(node_id)` defaults to section 0; anything else no-ops.
    /// - The editor opens via `open_text_edit`. NodeEdit mode stays
    ///   active; closing the editor (commit or cancel) returns to
    ///   `NodeEdit` mode (not `Default` — `ExitMode` does that).
    ///
    /// Sits in [`InputContext::NodeEdit`] so binding it to `Enter`
    /// at the NodeEdit context doesn't shadow the same key at the
    /// Document level (which is bound to `EnterNodeEdit`).
    #[action(context = NodeEdit, wasm = NativeOnly, destructive)]
    EnterSectionEdit,
    /// Open (or toggle) the CLI console.
    #[action(context = Document, wasm = NativeOnly)]
    OpenConsole,
    /// Save the currently-open mindmap document to its bound file path.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    SaveDocument,
    /// Copy the focused component's clipboard representation.
    /// **WASM:** the underlying `clipboard::write_clipboard` is a
    /// log-and-no-op stub today (async-clipboard integration
    /// pending). The Action is classified `Compatible` because it
    /// doesn't crash, but the user-visible behavior is "nothing
    /// happens." Tracked in `WASM_CONVERGENCE.md`.
    #[action(context = Document, wasm = Compatible, destructive)]
    Copy,
    /// Paste the system clipboard's text content into the focused component.
    /// **WASM:** same stub posture as `Copy` — `read_clipboard`
    /// returns `None`.
    #[action(context = Document, wasm = Compatible, destructive)]
    Paste,
    /// Cut: copy then clear the focused component's clipboard representation.
    /// **WASM:** same stub posture as `Copy`.
    #[action(context = Document, wasm = Compatible, destructive)]
    Cut,
    /// Enter resize mode on the current selection. Resolves the
    /// selection into a `ResizeTarget` (see
    /// `application::app::interaction_mode`):
    /// - `Single(node)` → `Resize { target: Node(node_id) }`.
    /// - `Section(s)` / `SectionRange` with `s.size == Some(_)` →
    ///   `Resize { target: Section { node_id, section_idx } }`.
    /// - `Section` with `size == None` (fill-parent) → no-op + log
    ///   (None-sized sections have no AABB to stretch).
    /// - `Multi` / `MultiSection` / `Edge*` / `None` → no-op + log.
    ///
    /// On success the active node / section emits 8 resize handles
    /// (NW, N, NE, E, SE, S, SW, W); anchor drag transitions
    /// `DragState` through the existing throttled-resize gestures.
    /// `ExitMode` (Esc) returns to `Default`.
    ///
    /// **WASM: NativeOnly until the resize gesture is wired
    /// cross-platform.** The mode flip itself is target-agnostic
    /// (the cross-platform `apply_enter_resize_mode` arm could
    /// run on WASM today), but WASM has no `DragState`, no
    /// throttled-drag pipeline, and no handle hit-test in
    /// `run_wasm/event_mouse_click.rs` — so a flip on WASM would
    /// render handles the user can't use.    /// (no half-features), the Action is `NativeOnly` until Batches
    /// 4 / 7 land the WASM gesture pipeline. Reclassification is a
    /// one-line change at that point.
    ///
    /// Non-destructive — flips a mode bit and triggers a scene
    /// rebuild, no document mutation. The actual resize commit
    /// happens later via `set_node_aabb` / `set_section_aabb` on
    /// drag release; that path is gated separately.
    #[action(context = Document, wasm = NativeOnly)]
    EnterResizeMode,
    /// Start a corner-anchored fast-resize gesture from the
    /// active right-button press. Threshold-cross arm in
    /// `event_cursor_moved.rs` dispatches this Action when the
    /// `DragState::PendingRight` press has moved past the drag
    /// threshold; the dispatch arm reads the press-time hit
    /// from a `DispatchHit` payload, computes the corner anchor
    /// via `infer_resize_anchor`, and transitions
    /// `DragState::PendingRight → Throttled(NodeResize |
    /// SectionResize)` with the chosen `ResizeHandleSide`. The
    /// release path on the right-button (Commit 3) finalizes
    /// via `set_node_aabb` / `set_section_aabb`.
    ///
    /// **Destructive on commit** (the release at the end of the
    /// drag writes through), but the *Action itself* is just a
    /// gesture-start signal — it doesn't mutate the document.
    /// User-tier-only macro privilege mirrors the EnterNodeEdit /
    /// EnterSectionEdit split: the gesture starts a
    /// destructive flow, even though the start itself is
    /// non-mutating.
    ///
    /// **WASM: NativeOnly** — the gesture relies on the
    /// `DragState` machinery in `event_mouse_click.rs` /
    /// `event_cursor_moved.rs` which is `cfg(not(target_arch =
    /// "wasm32"))`. It ships bound to `Ctrl+RightDrag` alone: the
    /// `TwoFingerDrag` touch peer it used to carry is gone, because
    /// two fingers on a canvas now drive the camera directly. WASM
    /// gets fast-resize when the browser gains a drag-state
    /// machine.
    ///
    /// Marked `destructive` so the macro privilege gate
    /// (`SourceTier::allows_action`) blocks System / Plugin /
    /// Project tiers; only User-tier macros (the user's keybind
    /// config + interactive console) can fire it.   
    #[action(context = Document, wasm = NativeOnly, destructive)]
    FastResizeStart,

    // ── Console ──────────────────────────────────────────────────
    /// Close the console (two-tier: dismiss popup first, then close).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleClose,
    /// Submit the current console input line for execution.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleSubmit,
    /// Cycle tab completions.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleTabComplete,
    /// Walk history backward / navigate completion popup upward.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleHistoryUp,
    /// Walk history forward / navigate completion popup downward.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleHistoryDown,
    /// Move cursor one grapheme left.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleCursorLeft,
    /// Move cursor one grapheme right.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleCursorRight,
    /// Move cursor to start of input.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleCursorHome,
    /// Move cursor to end of input.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleCursorEnd,
    /// Delete grapheme before cursor.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleDeleteBack,
    /// Delete grapheme after cursor.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleDeleteForward,
    /// Insert a literal space (winit delivers Space as Named, not Character).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleInsertSpace,
    /// Clear the current input line (shell Ctrl+C muscle-memory).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleClearLine,
    /// Jump cursor to start of line (shell Ctrl+A).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleJumpStart,
    /// Jump cursor to end of line (shell Ctrl+E).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleJumpEnd,
    /// Kill from cursor to start of line (shell Ctrl+U).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleKillToStart,
    /// Kill the word before cursor (shell Ctrl+W).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleKillWord,
    /// Scroll the scrollback window up by one line (Shift+Up).
    /// Plain Up still walks command history.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleScrollUp,
    /// Scroll the scrollback window down by one line (Shift+Down).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleScrollDown,
    /// Scroll the scrollback window up by one page
    /// (`MAX_CONSOLE_SCROLLBACK_ROWS` lines, PgUp).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleScrollPageUp,
    /// Scroll the scrollback window down by one page (PgDn).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleScrollPageDown,
    /// Pin the scrollback window at the bottom (newest line trailing,
    /// Shift+End).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleScrollEnd,
    /// Pin the scrollback window at the top (oldest reachable line,
    /// Shift+Home).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleScrollHome,

    // ── Color Picker ─────────────────────────────────────────────
    /// Cancel the color picker (contextual mode only; ignored in standalone).
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerCancel,
    /// Commit the current color (contextual: close; standalone: apply to selection).
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerCommit,
    /// Nudge hue −15°.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerNudgeHueDown,
    /// Nudge hue +15°.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerNudgeHueUp,
    /// Nudge saturation −0.1.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerNudgeSatDown,
    /// Nudge saturation +0.1.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerNudgeSatUp,
    /// Nudge value −0.1.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerNudgeValDown,
    /// Nudge value +0.1.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerNudgeValUp,

    // ── Label Editor ─────────────────────────────────────────────
    /// Cancel the inline label editor (discard changes).
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditCancel,
    /// Commit the inline label editor.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditCommit,

    // ── Text Editor ──────────────────────────────────────────────
    /// Cancel the inline text editor (discard changes).
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCancel,

    // ─────────────────────────────────────────────────────────────
    // ── Mouse-gesture Actions (Document context) ────────────────
    // The mouse handler synthesizes the gesture's canonical key
    // name (see `bind::gesture_key_name`) and feeds it through the
    // same `action_for_context` lookup as keyboard input. Every
    // gesture below can be bound to a key, and every keyboard
    // binding below can be bound to a gesture.
    // ─────────────────────────────────────────────────────────────
    /// Default-bound to `DoubleClick`. Dispatches by what the click hit:
    /// `Node` → open text editor, `PortalMarker`/`PortalText` → pan to
    /// partner endpoint, `EdgeLabel` → open inline label editor,
    /// `Empty` → fire `CreateOrphanNodeAndEdit` if it's bound (default
    /// off — the gesture is intentionally unbound for empty-canvas
    /// double-clicks).
    ///
    /// **WASM: mixed-branch**, classified `NativeOnly` under the "ANY
    /// NativeOnly branch" rule. Both targets resolve the gesture
    /// through this table and run one shared body
    /// (`cross_dispatch::pointer`), so rebinding or unbinding
    /// `double_click_activate` takes effect in the browser too. Three
    /// of the four routes are fully cross-platform; the `EdgeLabel`
    /// route commits the selection on both targets and then opens the
    /// single-line editor on native only, since the browser has no
    /// single-line editor yet. Same shape as `EditSelection`.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    DoubleClickActivate,
    /// Create an unattached node at the cursor and immediately open
    /// its text editor with an empty buffer. Sibling of `CreateOrphanNode`
    /// which only creates and selects (no editor). Default unbound.
    #[action(context = Document, wasm = Compatible, destructive)]
    CreateOrphanNodeAndEdit,
    /// Continuous left-button drag on empty canvas → camera pan.
    /// Default-bound to `LeftDrag` and `MiddleClick`. The dispatcher
    /// enters `DragState::Panning` on press and exits on release.
    #[action(context = Document, wasm = NativeOnly)]
    PanCanvas,

    // ── Navigation / camera (Document context) ──────────────────
    /// Zoom the camera in by one step. Default-bound to `WheelUp`.
    #[action(context = Document, wasm = Compatible)]
    ZoomIn,
    /// Zoom the camera out by one step. Default-bound to `WheelDown`.
    #[action(context = Document, wasm = Compatible)]
    ZoomOut,
    /// Reset the camera zoom to 1.0.
    #[action(context = Document, wasm = Compatible)]
    ZoomReset,
    /// Fit the entire mindmap tree to the viewport.
    #[action(context = Document, wasm = Compatible)]
    ZoomFit,
    /// Pan the camera north (up) by one step.
    #[action(context = Document, wasm = Compatible)]
    PanCameraNorth,
    /// Pan the camera south (down) by one step.
    #[action(context = Document, wasm = Compatible)]
    PanCameraSouth,
    /// Pan the camera east (right) by one step.
    #[action(context = Document, wasm = Compatible)]
    PanCameraEast,
    /// Pan the camera west (left) by one step.
    #[action(context = Document, wasm = Compatible)]
    PanCameraWest,
    /// Center the camera on the centroid of the current selection.
    #[action(context = Document, wasm = Compatible)]
    CenterOnSelection,
    /// Jump the camera + selection to the document's root node.
    #[action(context = Document, wasm = Compatible)]
    JumpToRoot,

    // ── Selection (Document context) ────────────────────────────
    /// Select every node in the document.
    #[action(context = Document, wasm = Compatible)]
    SelectAll,
    /// Clear the current selection.
    #[action(context = Document, wasm = Compatible)]
    DeselectAll,
    /// Invert the current selection (selected ↔ unselected).
    #[action(context = Document, wasm = Compatible)]
    InvertSelection,
    /// Select the parent of the currently-selected single node.
    #[action(context = Document, wasm = Compatible)]
    SelectParent,
    /// Select the first child of the currently-selected single node.
    #[action(context = Document, wasm = Compatible)]
    SelectChild,
    /// Select the next sibling of the currently-selected single node.
    #[action(context = Document, wasm = Compatible)]
    SelectNextSibling,
    /// Select the previous sibling of the currently-selected single node.
    #[action(context = Document, wasm = Compatible)]
    SelectPrevSibling,

    // ── TextEdit cursor primitives (TextEdit context) ───────────
    /// Move cursor one grapheme left.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorLeft,
    /// Move cursor one grapheme right.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorRight,
    /// Move cursor one visual line up.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorUp,
    /// Move cursor one visual line down.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorDown,
    /// Jump cursor to the start of the current line.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorHome,
    /// Jump cursor to the end of the current line.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorEnd,

    // ── TextEdit shift-select cursor primitives (TextEdit
    // context). Same as their non-`Select` siblings, but they
    // additionally seed the editor's `selection_anchor` (if
    // unset) so the (anchor, cursor) pair defines a sub-range
    // that lifts to `SelectionState::SectionRange` on close.
    /// Extend selection one grapheme left.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorLeftSelect,
    /// Extend selection one grapheme right.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorRightSelect,
    /// Extend selection one visual line up.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorUpSelect,
    /// Extend selection one visual line down.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorDownSelect,
    /// Extend selection to the start of the current line.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorHomeSelect,
    /// Extend selection to the end of the current line.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorEndSelect,

    /// Move cursor one word left.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditWordLeft,
    /// Move cursor one word right.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditWordRight,
    /// Delete the grapheme before the cursor.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditDeleteBack,
    /// Delete the grapheme at / after the cursor.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditDeleteForward,
    /// Delete from cursor back to the start of the current word.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditDeleteWordBack,
    /// Delete from cursor forward through the current word.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditDeleteWordForward,
    /// Commit the editor's buffer to the model and close. Default unbound
    /// (Enter is literal in the multi-line node editor).
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCommit,

    // ── LabelEdit cursor primitives (LabelEdit context) ─────────
    /// Move cursor one grapheme left in the label/portal-text editor.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditCursorLeft,
    /// Move cursor one grapheme right.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditCursorRight,
    /// Jump cursor to the start of the buffer.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditCursorHome,
    /// Jump cursor to the end of the buffer.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditCursorEnd,
    /// Delete the grapheme before the cursor.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditDeleteBack,
    /// Delete the grapheme at / after the cursor.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditDeleteForward,

    // ── Console-verb Actions (Document context) ─────────────────
    /// Open the glyph-wheel color picker as a standalone palette.
    /// Mirrors `color picker on`.
    #[action(context = Document, wasm = NativeOnly)]
    OpenColorPicker,
    /// Close the glyph-wheel color picker. Mirrors `color picker off`.
    #[action(context = Document, wasm = NativeOnly)]
    CloseColorPicker,
    /// Open the inline label editor on the currently-selected edge.
    /// Mirrors `label edit`.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    LabelEditOnSelection,
    /// Toggle the FPS overlay on/off. Mirrors `fps on` ↔ `fps off`.
    #[action(context = Document, wasm = Compatible)]
    ToggleFps,
    /// Toggle the FPS overlay's debug variant. Mirrors `fps debug` ↔
    /// `fps off`.
    #[action(context = Document, wasm = Compatible)]
    ToggleFpsDebug,
    /// Replace the current document with a fresh blank one. Mirrors
    /// `new` (no path).
    #[action(context = Document, wasm = NativeOnly, destructive)]
    NewDocument,

    // ── Parametric console-verb Actions (Document context) ──────
    // Each wraps a parameterised console verb so the user can bind
    // the verb directly without authoring a macro. Free-form
    // `String` payloads are parsed at dispatch time; bad values
    // emit a warn-log and the dispatch returns `Handled` (no
    // scrollback — Action arms have no scrollback surface).
    /// Mirror `anchor from=<side> to=<side>` on the selected edge.
    /// Sides: `auto|top|right|bottom|left`. Single-edge selection.
    #[action(context = Document, wasm = Compatible)]
    SetEdgeAnchor { from: String, to: String },
    /// Mirror `body glyph=<dot|dash|double|wave|chain>` on the
    /// selected edge.
    #[action(context = Document, wasm = Compatible)]
    SetEdgeBodyGlyph(String),
    /// Mirror `border <field>=<value>` on the selected node(s).
    /// Single kv per binding (multi-kv border edits stay
    /// console-only). Field names: `preset|font|size|color|palette|
    /// field|padding|top|bottom|left|right|tl|tr|bl|br`.
    ///
    ///calls for replacing this with per-field
    /// variants (`SetBorderPreset` / `SetBorderColor` / ...)
    /// for keybind ergonomics — `SetBorderField` stays as the
    /// generic catch-all for the kv form and for callers that
    /// want one-binding-many-fields semantics; deletion is a
    /// follow-up breaking-change commit pending the full
    /// parametric-binding migration.
    #[action(context = Document, wasm = Compatible)]
    SetBorderField { field: String, value: String },
    /// Mirror `border preset cycle` — advance the selected
    /// node(s)' border preset to the next entry in
    /// `BORDER_PRESETS`, wrapping at the end. No payload, so
    /// bindable to a single key for one-press preset cycling, as
    /// `cycle_border_preset`. This doc claimed that from the day
    /// the variant landed and the config field did not exist until
    /// #32 — the drift class the `keybind_surface!` table closes.
    #[action(context = Document, wasm = Compatible)]
    CycleBorderPreset,
    /// Mirror `border toggle` — flip `style.show_frame` per
    /// selected node. No payload (each node is toggled
    /// independently), so bindable to a single key for one-press
    /// border-on/off cycling, as `toggle_border_visible`. Same
    /// missing-field history as [`Action::CycleBorderPreset`].
    #[action(context = Document, wasm = Compatible)]
    ToggleBorderVisible,
    /// Stage a single-kv border preview against the live
    /// selection. `target_kind: BorderPreviewTargetKind` is a
    /// typed enum (kebab-case-serialized: `node` / `section` /
    /// `canvas-border` / `canvas-sf` / `canvas-sf-focused`)
    /// that picks the committing setter `commit_border_preview`
    /// will route to; replaces the prior stringly-typed
    /// discriminator that could accept typos at runtime. Single
    /// kv per binding (preview-set keybinds; multi-kv preview
    /// stays console-only). Mirrors the `<verb> preview
    /// <field>=<value>` console path without the model write.
    ///
    /// **`Section` target requires a section-shaped selection**
    /// (`Section` / `SectionRange` / `MultiSection`). On
    /// `Single(node_id)` the resolver in
    /// `apply_set_border_preview` falls through (no `section=K`
    /// payload exists on the Action variant) and the keybind
    /// becomes a no-op. The console verb path requires `section=K`
    /// to disambiguate; the keybind has no kv side-channel so
    /// authors who want a section preview keybind should bind it
    /// to a navigation key that lands a section-shaped selection
    /// first, or use the console verb.
    #[action(context = Document, wasm = Compatible)]
    SetBorderPreview {
        target_kind: BorderPreviewTargetKind,
        field: String,
        value: String,
    },
    /// Commit the active border preview through the matching
    /// committing setter and clear the preview slot. No-op when
    /// no preview is active.
    #[action(context = Document, wasm = Compatible)]
    CommitBorderPreview,
    /// Discard the active border preview without writing the
    /// model. No-op when no preview is active. Default-bound to
    /// **nothing** in `KeybindConfig::default()` — Esc cancels
    /// previews through `Action::ExitMode`'s body (which calls
    /// `cancel_border_preview()` first and short-circuits when a
    /// preview was canceled). The chain lives inside ExitMode
    /// because the keybind resolver maps `(context, key) →
    /// Action` deterministically and can't fall through. Bind
    /// this entry to opt out of the chain (e.g. preview-cancel
    /// on a different key while leaving Esc on plain ExitMode).
    #[action(context = Document, wasm = Compatible)]
    CancelBorderPreview,
    /// Mirror `cap from=<arrow|circle|diamond|none> to=<...>` on the
    /// selected edge.
    #[action(context = Document, wasm = Compatible)]
    SetEdgeCap { from: String, to: String },
    /// Mirror `color <axis>=<color>` on the current selection.
    /// `axis` picks the field group (background, text, or border);
    /// `value` is the user-facing color string (`#rrggbb`,
    /// `var(--name)`, palette key, etc.) which the verb-core parses.
    #[action(context = Document, wasm = Compatible)]
    SetColor { axis: ColorAxis, value: String },
    /// Mirror `edge type=<cross_link|parent_child>` on the selected
    /// edge.
    #[action(context = Document, wasm = Compatible)]
    SetEdgeType(String),
    /// Mirror `edge display_mode=<line|portal>` on the selected edge.
    #[action(context = Document, wasm = Compatible)]
    SetEdgeDisplayMode(String),
    /// Mirror `edge reset=<straight|curve|style|position>` on the
    /// selected edge.
    #[action(context = Document, wasm = Compatible)]
    ResetEdge(String),
    /// Mirror `font set <family>` on the current selection. Unknown
    /// family names silently no-op (the verb path surfaces a typed
    /// error; the Action arm has no scrollback).
    #[action(context = Document, wasm = Compatible)]
    SetFontFamily(String),
    /// Mirror `font <slot>=<pt>` on the current selection.
    /// `slot` picks `size|min|max`; `value` is the raw pt string
    /// parsed at dispatch time (non-finite or non-positive values
    /// silently no-op). `min` / `max` are selection-aware:
    /// applicable to edge / edge-label / portal-text channels;
    /// nodes have no screen-space clamp and no-op silently.
    #[action(context = Document, wasm = Compatible)]
    SetFont { slot: FontSlot, value: String },
    /// Mirror `label text=<text>` on the selected edge / portal
    /// label. Empty payload clears the label.
    #[action(context = Document, wasm = Compatible)]
    SetEdgeLabelText(String),
    /// Mirror `label position=<start|middle|end>` on the selected
    /// line-mode edge. Portal selections silently no-op (they use
    /// the `position_t=<f32>` shape, not named anchors).
    #[action(context = Document, wasm = Compatible)]
    SetEdgeLabelPosition(String),
    /// Mirror `spacing value=<tight|normal|wide|<float>>` on the
    /// selected edge.
    #[action(context = Document, wasm = Compatible)]
    SetSpacing(String),
    /// Mirror `zoom <bound>=<zoom|unset>` on the current selection.
    /// `bound` picks `min|max`; `value` is `"unset"`, `""`, or a
    /// positive finite float string. Inverted bounds (`min > max`)
    /// silently no-op.
    #[action(context = Document, wasm = Compatible)]
    SetZoom { bound: ZoomBound, value: String },
    /// Mirror `zoom clear` — drop both `min_zoom_to_render` and
    /// `max_zoom_to_render` on the current selection. Unit
    /// variant — no payload.
    #[action(context = Document, wasm = Compatible)]
    ClearZoom,
    /// Mirror `section move dx=<dx> dy=<dy>` — nudge the
    /// selected section by `(dx, dy)` canvas units. Keybind /
    /// macro path for the per-frame-safe section move. AABB
    /// rejection on overflow surfaces as a `log::warn!` and
    /// no-op.
    //
    // Stringly-typed payload convention: f64 / usize fields on
    // `Action` variants ride as `String` because the enum
    // derives Hash + Eq + Serialize / Deserialize across every
    // variant, and f64 doesn't `Hash`. Dispatch arms parse
    // them. This applies to every `Set/Add/Split/...Section*`
    // variant and to any future numeric-payload Action.
    #[action(context = Document, wasm = Compatible)]
    SetSectionOffsetDelta { dx: String, dy: String },
    /// Mirror `section resize w=<w> h=<h>` — pin the selected
    /// section's size to `(w, h)`. Same AABB validation as the
    /// verb path. `w` / `h` are parsed at dispatch time.
    #[action(context = Document, wasm = Compatible)]
    SetSectionSizeAbs { w: String, h: String },
    /// Mirror `section resize fill` — flip the selected section
    /// back to fill-parent (`size = None`). Renamed from `none`
    /// in Batch 5 (the prior literal misleadingly read as "remove
    /// the section"); this Action's name stays `FillParent` for
    /// keybind-stability.
    #[action(context = Document, wasm = Compatible)]
    SetSectionSizeFillParent,
    /// Mirror `section move x=<x> y=<y> [section=<idx>]` — pin the
    /// selected section's `offset` to `(x, y)`. The absolute
    /// setter, distinct from the `dx` / `dy` delta form.
    ///
    /// Bindable as `set_section_offset_abs`. It was macro-only
    /// until #32, for no reason its neighbours did not share: the
    /// delta form had a config field and the absolute one did not.
    #[action(context = Document, wasm = Compatible)]
    SetSectionOffsetAbs { x: String, y: String },
    /// Mirror `section text "<text>" [runs=preserve|clear]`.
    /// `runs_mode = "preserve"` clips existing runs to the new text
    /// length; `"clear"` collapses to a single run. Destructive.
    #[action(context = Document, wasm = Compatible, destructive)]
    SetSectionText { text: String, runs_mode: String },
    /// Mirror `section add [at=<idx>] [text="<text>"]`. `at = ""`
    /// appends; `"K"` inserts at K. `text = ""` makes an empty
    /// section. Destructive.
    #[action(context = Document, wasm = Compatible, destructive)]
    AddSection { at: String, text: String },
    /// Mirror `section delete [section=<idx>]`. Errors when the
    /// node has only one section — the model invariant.
    /// Destructive.
    #[action(context = Document, wasm = Compatible, destructive)]
    DeleteSection,
    /// Mirror `section split at=<grapheme>`. `"K"` splits at
    /// grapheme K; `at_grapheme = ""` is **rejected** with a
    /// warn-log, matching the verb, which requires `at=` because
    /// split-at-end-of-text silently creates an empty suffix
    /// section nobody wants. The field name disambiguates from
    /// `AddSection { at }`'s section-vector index (different
    /// units). Destructive.
    #[action(context = Document, wasm = Compatible, destructive)]
    SplitSection { at_grapheme: String },
    /// Mirror `open <path>` — replace the current document with the
    /// one loaded from `path`. **NativeOnly** + **destructive**:
    /// touches the filesystem. Denylisted for non-User macro tiers
    /// — a hostile mindmap must not be able to load arbitrary
    /// content as the active document.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    OpenDocument(String),
    /// Mirror `save <path>` — write the current document to `path`
    /// and rebind. **NativeOnly** + **destructive**: writes to the
    /// filesystem; a hostile macro could overwrite arbitrary files.
    /// Denylisted for non-User macro tiers.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    SaveDocumentAs(String),
    /// Mirror `new <path>` — start a fresh document and bind it to
    /// `path` (writes a blank file there immediately).
    /// **NativeOnly** + **destructive**: writes to the filesystem.
    /// Denylisted for non-User macro tiers.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    NewDocumentAt(String),
}

// `Action::is_destructive`, `::context`, `::wasm_compatibility` are
// generated by `mandala_derive::ActionClassify` (each forwards to
// `ActionKind::from(self).method()`); see the derive's emit body in
// `lib/mandala_derive/src/lib.rs`. The canonical classifier lives on
// the discriminant enum so its match doesn't need to destructure
// payloads; the delegates exist so callers reach for whichever shape
// is closer to hand.

/// Cross-platform compatibility classification for an [`Action`].
///
/// The Mandala application targets both native (winit + desktop wgpu)
/// and WASM (browser canvas + web wgpu). Today native has every
/// modal and state machine; WASM has a curated subset (document,
/// renderer, node text editor, mouse, keyboard). The
/// [`Action::wasm_compatibility`] method classifies each variant
/// based on which target's machinery it requires.
///
/// **Where this matters.** `run_wasm/event_keyboard.rs` and
/// `run_wasm/event_mouse_click.rs` filter on this so a key
/// bound to e.g. `Action::OpenConsole` silently no-ops in the
/// browser instead of triggering a panic
/// at the dispatch site. As WASM gains its own version of the
/// missing modals (see `WASM_CONVERGENCE.md`), variants migrate
/// from `NativeOnly` to `Compatible` one at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WasmCompatibility {
    /// Action works identically on native and WASM today. Its
    /// dispatch arm reads / writes only state both targets have:
    /// `MindMapDocument`, `Renderer`, `TextEditState`, mouse
    /// gestures, the macro registry. Safe to fire from a WASM
    /// `dispatch_action_for_wasm` once that path is built.
    Compatible,
    /// Action requires a native-only system not yet ported to
    /// WASM (console, color picker, inline label / portal-text
    /// editors, `DragState`, filesystem `save`). The `InteractionMode`
    /// enum itself is cross-platform; only the click-handler paths
    /// that read it on native still gate Reparent / Connect arms here.
    /// Currently a no-op on WASM; the convergence path is to
    /// either port the underlying system or surface a WASM-
    /// specific equivalent and flip the classification to
    /// `Compatible`. Tracked in `WASM_CONVERGENCE.md`.
    NativeOnly,
}

/// The **mixed-branch set**: Actions whose dispatch arm has both a
/// cross-platform slice and a native-only leftover. One list, in one
/// place, because two consumers have to agree on it and previously
/// did not:
///
/// - `app::dispatch::action_core::lift_mixed_branch_for_wasm_macro`
///   decides, per member, whether an `Unhandled` from the
///   cross-platform dispatcher means "WASM did everything it can"
///   (so a WASM macro's `any_ran` must bump) or "nothing ran".
/// - `keybinds::tests::test_wasm_compatibility_mixed_branch_actions_are_native_only`
///   pins each member's classification.
///
/// The second element is that classification, and it is **not** the
/// same for every member — which is why two hand-written lists
/// drifted. [`Action::ExitMode`] is mixed-branch and `Compatible`:
/// its native leftover is a *step* (clearing `hovered_node` so the
/// Reparent / Connect overlay rebuilds), not a branch that reaches
/// native-only state. The other three each have a whole branch
/// ending in a native-only editor, so the "ANY NativeOnly branch ⇒
/// NativeOnly" rule classifies them `NativeOnly`. Carrying the
/// expected classification per member keeps that difference stated
/// rather than lost, and makes a re-classification fail a test
/// instead of quietly widening the set.
///
/// Adding a member here obliges both consumers: the keybind test
/// asserts its classification, and the lift's `debug_assert` fires in
/// any test build if the member has no verdict arm.
// Consumer is `run_wasm/`; the host build reaches this only from
// `#[cfg(test)]`, so arm the lint on wasm32 and silence it here
// rather than blanket-allowing on both targets — if the browser
// caller goes away, wasm32 says so.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) const MIXED_BRANCH_ACTIONS: [(Action, WasmCompatibility); 4] = [
    // Native leftover: the `hovered_node` clear for the target-picker
    // overlay rebuild. Cross-platform slice: border-preview cancel,
    // `last_click` clear, `InteractionMode` reset + rebuild.
    (Action::ExitMode, WasmCompatibility::Compatible),
    // EdgeLabel + Portal* selection branches reach native-only
    // editors; the node-scoped branch is cross-platform.
    (Action::EditSelection, WasmCompatibility::NativeOnly),
    (Action::EditSelectionClean, WasmCompatibility::NativeOnly),
    // The EdgeLabel route reaches `open_single_line_edit`; the node,
    // portal and empty-canvas routes are cross-platform.
    (Action::DoubleClickActivate, WasmCompatibility::NativeOnly),
];
