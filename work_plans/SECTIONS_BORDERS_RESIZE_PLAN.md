# Sections, Borders & Resize — UX Overhaul Plan

> **Status.** Under development. This document is the contract that
> follow-up sessions execute against. Every batch is a discrete commit
> (or a small commit chain) that leaves the tree in `./test.sh`-green
> shape. Tick boxes flip as items land.
>
> **Relationship to other plans.** `REFACTOR_PLAN.md` is the
> infrastructure-debt plan; some of its deferred items (DragState
> Pending refactor, ModalState carve-out, KeybindConfig derive) sit
> adjacent to this one. Where they overlap, this plan calls them out
> and either depends on the work or fences it as out-of-scope.
>
> **Authoring discipline.** Every change must respect:
> - `CODE_CONVENTIONS.md` — single dispatch funnel, mutation-first,
>   model/view separation, glyph-only, cross-platform first-class,
>   canonical or exemplary, drive-by fixes, no half-features
>   (§5), no backwards-compat shims (§10).
> - `lib/baumhard/CONVENTIONS.md` — mutation-not-rebuild, grapheme
>   awareness, arena discipline, no-unsafe.
> - `format/sections.md` and `format/border-patterns.md` — data-model
>   contracts the UX renders.
> - The "all interaction is an `Action`" stance from CLAUDE.md and
>   CONCEPTS.md §5: **no ad-hoc mode toggles, no mouse-button
>   side-effects without a corresponding `Action` variant**.

## 0. Why this exists

Mandala has accreted three closely-related UX cliffs around the
post-section-refactor data model:

1. **Borders are powerful but unteachable.** The `border` console
   verb has 16 kv keys, a side-pattern grammar (`prefix(fill)suffix`)
   the user must read `format/border-patterns.md` to use, silently
   drops kvs after `border on/off/reset`, conflates `style.frame_color`
   and `style.border.color` between `color border=…` and `border
   color=…`, has a flat completion list with no grouping, and forces
   `preset=custom` as a sentinel before per-side glyphs apply. A
   curious user can't discover what works by typing.

2. **Sections are expressive but invisible.** A multi-section node is
   structurally a stack of independently-positioned text strata, but
   today every interaction defaults to "the node is the unit": the
   lasso doesn't multi-select sections, single-click on a section
   silently demotes selection to `Section`, the inline editor opens on
   `section_idx = 0` for any non-Section selection, and there is no
   user-facing surface for adding, removing, or splitting sections.
   The `section` console verb only `move`s and `resize`s — and it
   uses positional `<dx> <dy>` arguments that are inconsistent with
   every other kv-style verb. Multi-section nodes effectively cannot
   be authored in-app.

3. **Resize anchors fire by accident.** The 8 resize handles auto-emit
   on `Single` selection and on any `Some`-sized `Section` selection,
   sit at the AABB corners + edge midpoints with a 12px hit tolerance,
   and take precedence over the body in the threshold-cross priority
   chain. A user trying to grab a node by its corner (a natural
   target for "move") instead resizes it. The handle visibility is
   bound 1-to-1 to selection, so there is no signal differentiating
   "selected, ready to move" from "selected, ready to resize".

These are not three independent UX bugs. They share a missing
concept — **explicit modes** — and the absence of that concept
forces each subsystem to overload selection with mode semantics. The
fix is one small piece of new infrastructure (a unified
`InteractionMode`) followed by three coordinated UX retunes that
hang off it.

## 1. Guiding principles

These steer every decision below. Where two principles conflict, the
earlier one wins.

1. **Modes are explicit and reified.** A user is *always* in exactly
   one `InteractionMode`; transitions are `Action` variants; `Action`
   variants have keybind defaults, console verbs, and macro reach.
   Modes never appear via mouse-button side-effect alone.

2. **Selection is orthogonal to mode.** `SelectionState` says *what
   is targeted*; `InteractionMode` says *what targeting it means*.
   The two compose — e.g. `Single(node)` selection in `Default`
   mode is "this node is selected for movement / console targeting";
   `Single(node)` in `NodeEdit { node_id }` mode is "this node is
   the active mini-canvas".

3. **One selection-mode-vs-edit-mode rule per nesting level.**
   - Canvas-level: `Default` ≈ "selection mode for nodes"; `NodeEdit`
     ≈ "edit mode for the canvas-as-node-content".
   - Node-level: inside `NodeEdit`, the section is the unit;
     `Section` selection ≈ "selection mode for sections"; entering
     the text editor on a section ≈ "edit mode for that section's
     text".
   This nesting must be visually obvious (§3.5) and must apply
   consistently — the same mental model unlocks every level.

4. **All operations are Actions.** Every UX-facing command — enter
   a mode, exit a mode, toggle a sub-state, resize a node, pick a
   border preset — is a variant on `Action`. The console verb that
   triggers it, the keybind that triggers it, and the future GUI
   button that triggers it all dispatch the same Action. No
   second copy of the body lives in a handler.

5. **Mode-gated chrome is rendered by mode, not by selection.**
   Resize handles, section frames, target highlights — anything
   visual that signals "you can do X here right now" is a function of
   the active mode (and its target), never of `SelectionState`
   alone. This is the single change that fixes the auto-resize
   problem and makes the section UX legible.

6. **Reuse the existing seams, do not parallel-build.** The
   investigation identified concrete attachment points — the
   `SceneSelectionContext` handle gates, the `ResizeHandleSide::resolve_aabb`
   shared math, the `Flag::SectionRoot` discriminator, the
   `set_section_aabb` / `set_node_aabb` atomic setters, the
   `SectionPayload` clipboard surface, the unwired `Flag::Focused`
   and `Flag::Mutable` flags. New chrome and new behaviors hang off
   these. We do not build a second tree-pass, a second resize-math
   library, a second per-section data carrier.

7. **Single-section nodes preserve today's whole-node semantics.**
   `hit_test_target` already collapses single-section hits to
   `NodeContainer` (`/home/user/mandala/src/application/document/hit_test.rs:130-138`).
   The new mode-driven UX must keep this fold: a single-section node
   in `Default` mode behaves exactly like today, and entering
   `NodeEdit` on a single-section node is a no-op transition (or a
   short-circuit straight to `SectionEdit`). Any UX that visibly
   distinguishes "single-section in NodeEdit" must justify its cost.

8. **Mobile is a peer, not a fallback.** Every mode must have a
   touch-reachable transition (§5 in CODE_CONVENTIONS). For Resize
   mode in particular — where the desktop story relies on
   `Ctrl+RightDrag` — the touch story is two-finger pinch on a
   selected node, dispatched through the same `Throttled(NodeResize)`
   shell. Native first, web second is a sequencing choice; touch
   parity is a release gate.

## 2. Architecture overview

### 2.1 The three problems share one missing concept

Each of the three UX cliffs has the same shape: *a discrete behavior
that should be reified as a mode is implicit in either selection state
or mouse-button side-effect.*

| Problem | Implicit mode today | Where it lives |
|---|---|---|
| Border syntax is unteachable | "I'm authoring a border" | inferred from the verb prefix, no UI feedback |
| Section UX is invisible | "I'm editing this node's contents" | inferred from `TextEditState::Open`, opens at `section_idx=0` |
| Resize fires by accident | "I'm resizing this node" | implicit in selection — handles auto-emit |

The fix in each case is the same: **make the mode explicit, gate the
chrome on the mode, route transitions through `Action`**. That gives:

- A user signal: "you are in Resize mode" — UI shows handles, status
  bar shows mode name, Esc exits.
- An automation surface: macros and keybinds can enter / exit modes
  predictably.
- A console surface: `mode resize`, `mode node-edit`, etc. become
  natural verbs, and existing verbs (`section move`, `border
  preset=heavy`) compose with mode (e.g. `border preset=heavy` while
  in NodeEdit mode targets the active node, giving multi-modal
  authoring).

### 2.2 The InteractionMode enum

A new cross-platform module `src/application/app/interaction_mode.rs`
will define the unified mode enum. It absorbs `AppMode`'s three
existing variants (`Normal`, `Reparent`, `Connect`) and adds
`NodeEdit`, `Resize`. `SectionEdit` is **not** a mode in this enum —
it is represented by the existing `TextEditState::Open` modal-stealer,
which always nests inside `NodeEdit` (per principle §1.3 above).

```rust
// src/application/app/interaction_mode.rs

/// The active high-level interaction mode for the application. Drives:
/// - which clicks are absorbed (Reparent / Connect intercept the next click;
///   NodeEdit demotes whole-node clicks to section-clicks; Resize captures
///   any-quadrant click as a resize gesture).
/// - which mode-gated chrome is rendered (resize anchors, section frames,
///   target highlights).
/// - how `SelectionState` clicks resolve (e.g. NodeEdit mode reroutes
///   section-area clicks to set `Section`, while in `Default` they fold
///   to `Single`).
///
/// Cross-platform — replaces the native-only `AppMode`. The enum
/// carries no GPU handles and depends only on `String` ids and the
/// shared `ResizeTarget` value; both targets compile it.
///
/// `SectionEdit` (cosmic-text editing of one section's text) is *not* a
/// variant — it is carried by `TextEditState::Open { node_id, section_idx, ... }`
/// in the modal-stealer cascade. The invariant `TextEditState::Open` ⇒
/// `InteractionMode::NodeEdit { node_id }` (matching id) is enforced by
/// the `Action::EnterSectionEdit` arm and by `close_text_edit`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum InteractionMode {
    /// Default: selection + movement. Clicks select; drags move; no
    /// mode-gated chrome.
    Default,

    /// User is choosing a new parent for `sources`. The next left-click
    /// on a node attaches them; left-click on empty canvas promotes to
    /// root; Esc cancels. Replaces `AppMode::Reparent`.
    Reparent { sources: Vec<String> },

    /// User is drawing a new cross_link edge from `source`. The next
    /// left-click on a target node creates the edge; left-click on empty
    /// canvas cancels. Esc also cancels. Replaces `AppMode::Connect`.
    Connect { source: String },

    /// Editing the contents of a node — the node behaves as a
    /// "mini-canvas". Section clicks select sections (per
    /// `SelectionState::Section`); section drags move sections;
    /// section-text edits require entering `SectionEdit` (TextEditState).
    /// Click outside the node's overflow-aware AABB exits the mode and
    /// returns to `Default`. Esc also exits.
    NodeEdit { node_id: String },

    /// Resize anchors are visible on `target`. Anchor drag invokes the
    /// existing `Throttled(NodeResize)` / `Throttled(SectionResize)`
    /// gestures. Click on the body (not on a handle) exits the mode.
    /// Esc also exits.
    Resize { target: ResizeTarget },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResizeTarget {
    Node(String),
    Section { node_id: String, section_idx: usize },
}

impl InteractionMode {
    /// True if this mode wants to absorb the next left-click as a
    /// mode-specific gesture (Reparent / Connect / Resize body-exit).
    /// Drives the click-routing fork in `event_mouse_click.rs`.
    ///
    /// SUPERSEDED — this predicate was deleted by #41. It shipped
    /// with tests but never acquired a production caller: the click
    /// router in `event_mouse_click.rs` and `click.rs` matches
    /// `InteractionMode::Reparent` / `Connect` inline instead. Kept
    /// in this sketch because the sketch is the historical record of
    /// what the batch designed, not a description of today's tree.
    pub fn intercepts_left_click(&self) -> bool { ... }

    /// True if a click on a section-area should produce
    /// `SelectionState::Section` (NodeEdit) vs collapsing to
    /// `SelectionState::Single` (Default). Single source of truth
    /// for the click router's section-vs-node decision.
    pub fn click_resolves_to_section(&self, hit_node: &str) -> bool { ... }

    /// Which node should receive auto-resize-handle emission this frame,
    /// or None. Replaces the selection-driven gate in `document/mod.rs:520`.
    pub fn resize_handle_node(&self) -> Option<&str> { ... }

    /// Which section should receive auto-resize-handle emission this
    /// frame, or None.
    pub fn resize_handle_section(&self) -> Option<(&str, usize)> { ... }
}
```

### 2.3 Relationship to existing concepts

This subsection is a contract for how `InteractionMode` coexists with
the surfaces it didn't fully replace.

- **`SelectionState`** stays as-is structurally; only the *click
  routing* into it changes (§4.1). `Multi` / `MultiSection` /
  `SectionRange` etc. are unchanged. The `apply_tree_highlights`
  cyan-tint pipeline keeps its current shape; it's selection-driven,
  not mode-driven (selection still says *what is selected*, including
  inside NodeEdit).

- **Modal stealers** (`ConsoleState`, `ColorPickerState`,
  `SingleLineEditor`, `TextEditState`) stay as
  separate states. They steal *keystrokes*; modes drive *click
  routing and chrome*. The two systems are layered, not merged. This
  matches CODE_CONVENTIONS §3 carve-out for modal steals.

- **`AppMode`** is deleted. Its three variants migrate one-for-one
  into `InteractionMode`. The native-only cfg gate goes away (per
  principle §1.8 — cross-platform first-class). Reparent and Connect
  modes become available on WASM, which closes a long-standing gap
  noted in `WASM_CONVERGENCE.md`.

- **`DragState`** stays as-is. Its `Pending` / `Throttled(*)` machinery
  is mode-agnostic — entering `Resize` mode doesn't put the
  `DragState` into a special state; the mode just changes what *the
  next mouse press* will produce (a `Throttled(NodeResize)` regardless
  of which corner was clicked, in fast-resize mode). The press →
  pending → throttled cascade is unchanged.

- **`InputContext`** (the keybind context — `Document`, `Console`,
  `ColorPicker`, `LabelEdit`, `TextEdit`) gets one new variant:
  `NodeEdit`. Modes that intercept clicks but don't change keystroke
  routing (Reparent, Connect, Resize) keep their keybinds in
  `Document` context — they only need Esc to cancel, which can be
  handled by the existing `CancelMode` Action. NodeEdit needs its own
  keybind context for things like "Enter to enter SectionEdit on the
  selected section" without that key escaping to Document for other
  bindings.

- **`Flag::Focused` and `Flag::Mutable`** (currently defined but
  unused) are wired to mark mode targets. `Flag::Focused` sits on
  the section-area being edited (replaces the implicit
  `TextEditState::Open` lookup); `Flag::Mutable` sits on every
  section-area inside a `NodeEdit`-active node so the renderer can
  choose to draw the section frame. Both flags are pure data, no new
  fields.

- **`MouseGesture`** gets two new variants: `RightDrag` and
  `RightClick` (the latter reintroduced — the previous removal-as-
  half-feature comment in `keybinds/bind.rs:40-43` explicitly invites
  this). Touch gesture variants (`TwoFingerPinch`,
  `TwoFingerDrag`) are deferred to the touch story (§6.6).

### 2.4 What this plan is *not*

To set boundaries explicitly:

- This plan **does not redesign the data model**. `MindNode`,
  `MindSection`, `GlyphBorderConfig` are unchanged. The `format/`
  contracts are unchanged.
- This plan **does not change the renderer pipeline**. The
  flat-element passes, the tree builder, and the buffer caches are
  unchanged. Only the inputs to `SceneSelectionContext` change.
- This plan **does not introduce a GUI**. Buttons, toolbars, status
  bars are out of scope — but the new `Action` variants and the new
  modes are designed so that a GUI built later attaches with no
  further refactoring. That's the seam (per CODE_CONVENTIONS §7).
- This plan **does not migrate to a new keybind format** (per §10
  — no backwards-compat). It adds new fields with sensible defaults;
  user `keybinds.json` files stay valid.
- This plan **does not deliver custom mutations for sections beyond
  what already exists**. Section-level custom mutation routing (the
  `target_scope: SectionsOnly` path) is already in place and not
  changing.
## 3. The mode system

This section is the foundation everything else hangs off. It must
land first; later batches assume the mode infrastructure is in place.

### 3.1 New module: `src/application/app/interaction_mode.rs`

Cross-platform module (no `#[cfg]` gates). Contains:

- The `InteractionMode` enum (definition in §2.2 above).
- The `ResizeTarget` enum (definition in §2.2 above).
- The four `InteractionMode` predicate methods listed in §2.2.
- `Default` impl returning `InteractionMode::Default`.
- A `pub fn target_node_id(mode: &InteractionMode) -> Option<&str>`
  free helper that returns the mode's primary node target if any
  (used by the scene-rebuild path to drive cyan-tint chrome).

Tests live in the same file under `#[cfg(test)] mod tests`, per
`TEST_CONVENTIONS.md` policy on small pure-data modules.

### 3.2 State management

`InteractionMode` lives on `InitState` (native) and `WasmInputState`
(WASM), replacing the existing `app_mode: AppMode` field at:

- `/home/user/mandala/src/application/app/run_native.rs:210` —
  `pub(super) app_mode: AppMode` becomes
  `pub(super) interaction_mode: InteractionMode`.
- `/home/user/mandala/src/application/app/run_native_init.rs:231` —
  `app_mode: AppMode::Normal` becomes
  `interaction_mode: InteractionMode::Default`.
- `/home/user/mandala/src/application/app/run_wasm/mod.rs` — the
  equivalent field is added (today there is none, since AppMode is
  native-only). Initialised to `Default`.
- `/home/user/mandala/src/application/app/input_context.rs:53` and
  `:116` — `app_mode: &'a mut AppMode` becomes
  `interaction_mode: &'a mut InteractionMode`. Same in
  `input_context_core.rs`.
- The `split_borrow` method (`input_context.rs:109-144`) routes the
  borrow through.

Every existing `AppMode::Normal` reference in the codebase becomes
`InteractionMode::Default`; every `AppMode::Reparent { sources }`
becomes `InteractionMode::Reparent { sources }`; every
`AppMode::Connect { source }` becomes
`InteractionMode::Connect { source }`. This is a mechanical rename
across roughly 30 call sites — bounded by the existing
`AppMode` consumers.

### 3.3 Action variants for mode transitions

Add the following to `Action` in
`/home/user/mandala/src/application/keybinds/action/mod.rs`. Each
must include the `#[action(context, wasm)]` attributes per the derive
contract.

```rust
// Document context, NativeOnly today (NodeEdit / Resize use the
// modal-stealer pattern same as TextEdit; WASM lift comes in a
// follow-up batch — see §8 for sequencing).

/// Enter NodeEdit mode on the selected single node. With
/// `Single(node)` selection: enter `NodeEdit { node_id }`. With
/// `Section(s)` or `SectionRange { sel, .. }` selection: enter
/// `NodeEdit { s.node_id }` (the section selection is preserved
/// inside the mode). With any other selection (`Multi`,
/// `MultiSection`, `Edge*`, `None`): no-op + log::warn!.
///
/// Single-section nodes short-circuit: `EnterNodeEdit` directly opens
/// the text editor on section 0 (the only section), entering
/// `NodeEdit { node_id }` mode plus opening `TextEditState::Open`
/// in one transition. This preserves today's "Enter to type" UX
/// for legacy single-section maps.
#[action(context = Document, wasm = Compatible, destructive)]
EnterNodeEdit,

/// Exit NodeEdit / SectionEdit / Resize / Reparent / Connect modes
/// back to `Default`. Replaces `CancelMode` (which is renamed to
/// `ExitMode` for clarity — the alias keeps default keybind compat).
/// On exit:
/// - From `NodeEdit`: drop section selection, re-set
///   `SelectionState::Single(node_id)` so the previously-edited node
///   remains visually selected.
/// - From `SectionEdit`: route through `close_text_edit(commit=false)`
///   first (which lifts SectionRange when shift-select was active),
///   then transition to `NodeEdit { node_id }`. (i.e. one Esc moves
///   one nesting level out, two Escs return to canvas Default.)
/// - From `Resize`: drop the resize target; re-set
///   `SelectionState::Single(node_id)` (or Section).
/// - From `Reparent` / `Connect`: drop the captured sources / source.
///   Same body as today's `CancelMode`.
#[action(context = Document, wasm = NativeOnly)]
ExitMode,

/// Enter SectionEdit (i.e. open the text editor) on the active section
/// inside the current `NodeEdit { node_id }` mode. Preconditions:
/// - `InteractionMode::NodeEdit { node_id }` is active.
/// - `selection.selected_section()` returns `Some` on a section of
///   the active node, OR `Single(node_id)` is selected (in which case
///   default to section 0).
/// - `TextEditState::Closed`.
/// On success: opens `TextEditState::Open { node_id, section_idx, ... }`
/// via `open_text_edit` (the existing helper) and the mode stays at
/// `NodeEdit { node_id }`. Closing the editor (commit or cancel)
/// keeps mode at `NodeEdit`; `ExitMode` then exits NodeEdit too.
///
/// **Section discriminator.** `EnterSectionEdit` carries no payload
/// — the section is decided from selection state. To target a
/// specific section without first clicking it, use the console verb
/// `section edit <idx>`.
#[action(context = NodeEdit, wasm = NativeOnly, destructive)]
EnterSectionEdit,

/// Enter Resize mode on the selected node or section. Resolution:
/// - `Single(node)` → `Resize { target: Node(node_id) }`.
/// - `Section(s)` with `s.size == Some(_)` → `Resize { target: Section(...) }`.
/// - `Section(s)` with `s.size == None` (fill-parent) → no-op + log
///   (None-sized sections have no own AABB to stretch).
/// - Anything else → no-op + log.
#[action(context = Document, wasm = Compatible)]
EnterResizeMode,

/// Continuous gesture: hold-modifier+RightDrag to resize the node or
/// section under the cursor. Dispatched from the mouse handler when
/// `MouseGesture::RightDrag` (or modified `Ctrl+RightDrag`, etc.) is
/// pressed and a node body is hit. Rather than a discrete entry/exit,
/// this Action is the press-promotion hook: it transitions
/// `DragState::None → DragState::Throttled(NodeResize | SectionResize)`
/// with the side inferred from cursor quadrant relative to AABB
/// center (§6.5). The release path is the standard
/// `Throttled(NodeResize) → set_node_aabb` body, identical to
/// handle-driven resize.
#[action(context = Document, wasm = NativeOnly, destructive)]
FastResizeStart,
```

`Action::CancelMode` is **renamed** to `Action::ExitMode` for
semantic clarity (it cancels modes, but it's also the affirmative
"go back one level" action). The default keybind (`Escape`) stays.
Per CODE_CONVENTIONS §10 (no backwards-compat), there is no alias.
User keybinds carrying `cancel_mode` get a console hint at startup
and are migrated by the user.

### 3.4 Dispatch arms

Each new Action variant gets a dispatch arm in
`/home/user/mandala/src/application/app/dispatch/`. Following the
existing split between `action_core.rs` (cross-platform Compatible
arms) and `native.rs` (NativeOnly + DispatchHit-bearing arms):

- `EnterNodeEdit` is `Compatible` because it doesn't depend on
  filesystem or console state. The arm goes in `action_core.rs`,
  delegating to a new `cross_dispatch::lifecycle::apply_enter_node_edit`
  helper. Single-section short-circuit: if `node.sections.len() == 1`,
  the helper opens the text editor (calling the existing
  `apply_open_text_edit_on_single`) and sets mode to NodeEdit
  atomically.
- `ExitMode` is `NativeOnly` because the SectionEdit-exit branch
  routes through `close_text_edit`, which today is native-only. Once
  WASM gains text-edit parity (already on the WASM-convergence list),
  this can lift to `Compatible`. The arm replaces the existing
  `CancelMode` arm at `dispatch/native.rs:177-198`.
- `EnterSectionEdit` is `NativeOnly` (same reason — `open_text_edit`
  is native today). Arm in `dispatch/native.rs`.
- `EnterResizeMode` is `Compatible` (mode flip + scene rebuild
  only). Arm in `action_core.rs` → `cross_dispatch::lifecycle::apply_enter_resize_mode`.
- `FastResizeStart` is `NativeOnly` (DragState manipulation is
  native-only today). Arm in `dispatch/native.rs`. Takes `DispatchHit`
  to read the canvas position for anchor inference.

The existing `Action::EditSelection` arm at
`dispatch/action_core.rs:112-137` and `dispatch/native.rs:321-355`
is **rewritten** to dispatch through `EnterNodeEdit` for node-bearing
selections instead of opening the editor directly:

- `Single(node)` / `Section(s)` / `SectionRange` → equivalent to
  `EnterNodeEdit` (which short-circuits to the editor on
  single-section nodes, or stops at NodeEdit for multi-section,
  letting the user pick which section to edit).
- `EdgeLabel(s)` / `PortalLabel` / `PortalText` → opens the relevant
  inline editor (today's behavior, unchanged).
- `Multi` / `MultiSection` / `Edge` / `None` → no-op + log (today's
  behavior, unchanged).

`EditSelectionClean` mirrors `EditSelection` but passes `clean=true`
through to `open_text_edit` for the single-section short-circuit.

This change has user-visible behavior: pressing `Enter` on a
multi-section node now enters NodeEdit mode (showing section frames,
making sections clickable) rather than dropping straight into the
editor on section 0. The user then either clicks a section + presses
Enter again, or types `section edit 1` in the console.

### 3.5 Visual indicators

The mode must be discoverable. Each mode gets:

#### `Default` mode

No mode-gated chrome. Selection highlight (cyan tint on selected
node/section text) is the only feedback, matching today.

#### `Reparent` / `Connect` modes

Today's behavior stays: source nodes tinted orange
(`REPARENT_SOURCE_COLOR`); hovered candidate target tinted green
(`REPARENT_TARGET_COLOR`). No additional changes — these modes are
already discoverable via the highlight color change.

#### `NodeEdit { node_id }` mode

Three changes from today:

1. **Section frames render** for every `MindSection` of the active
   node. Frame is a thin glyph-drawn rectangle in
   `SELECTED_EDGE_COLOR` (the cyan we already use for selection
   highlights) that traces the section's effective AABB. Renders
   regardless of whether the section is `Some`-sized or
   fill-parent — a fill-parent section's frame coincides with the
   node's inner padding rectangle. Single-section nodes skip the
   frame (it would just be a duplicate of the border, and
   single-section short-circuits past NodeEdit anyway — see §3.4
   and §4.2).

2. **Inactive nodes dim**. Every `MindNode` other than the active
   one renders at 50% alpha for both text and chrome. This is the
   "you are inside this node" signal the user expects from a mode
   that reframes the canvas.

3. **A status-bar line** at the top of the canvas overlay reads
   `editing: <node-id> — section [N of M] <name?>` where N/M reports
   the active section index (selection-derived) and total. The bar
   is rendered as a `GlyphArea` in the `AppScene` overlay layer,
   following the same pattern as the FPS overlay. The bar disappears
   in `Default` mode.

#### `SectionEdit` (TextEditState::Open inside NodeEdit)

Today's behavior stays: cosmic-text caret rendered inside the
section's AABB; the editor's per-keystroke `apply_text_edit_to_tree`
delivers live preview. The status bar line updates to read
`editing text: <node-id>[<section_idx>]`.

The only addition: while `TextEditState::Open`, the `Flag::Focused`
flag is set on the active section-area's `GfxElement`. The renderer
honors this flag by drawing a slightly thicker / brighter section
frame than the inactive sections of the same node. (Currently
`Flag::Focused` is unused; this gives it its first consumer.)

#### `Resize { target }` mode

1. **Resize anchors render** on the targeted node or section
   (replacing the auto-emit-on-selection behavior — §6.2).
2. **A status-bar line** reads
   `resize: <target> — drag a corner or edge`. Esc exits.
3. **The body of the targeted node tints subtly** (10% cyan tint
   over the existing render) so the user sees which target is
   active when many nodes are visible.

### 3.6 Modal-stealer interaction

The existing modal-stealer cascade in
`/home/user/mandala/src/application/app/event_keyboard.rs:24-253`
gets one new branch:

```
1. Console open      (unchanged — outermost)
2. Color picker open (unchanged)
3. Label edit open   (unchanged)
4. Portal text edit open (unchanged)
5. Text edit open    (unchanged — implicit SectionEdit)
6. Mode-specific keys  (NEW — see below)
7. Document          (default fallthrough)
8. Macro lookup
9. Custom mutation lookup
```

Step 6 is new: when `interaction_mode != Default && text_edit not open`
and `action_for_context(InputContext::NodeEdit | Resize, ...)` returns
`Some(action)`, dispatch and return.

This adds a sixth `InputContext` variant (`NodeEdit`) and effectively
a seventh (`Resize`, but Resize doesn't need its own keybind table —
its only keybinds are `ExitMode` (Esc), which works in `Document` via
the existing fallthrough). So: only one new InputContext variant —
`NodeEdit` — in `keybinds/context.rs`.

`InputContext::NodeEdit::falls_through() == true` (per the cascade
contract — unmatched NodeEdit keys try Document, so e.g. `Ctrl+S`
still saves). This matches `ColorPicker`'s pattern.

### 3.7 Macro / privilege gate review

The new Action variants are reviewed against
`SourceTier::allows_action`:

- `EnterNodeEdit` is `destructive` (opens the editor, which is
  treated as destructive today — it can clear the section's text on
  the spot). User-tier-only.
- `ExitMode` is non-destructive. All tiers.
- `EnterSectionEdit` is `destructive` (same reasoning as
  `EnterNodeEdit`). User-tier-only.
- `EnterResizeMode` is non-destructive (mode flip + chrome change,
  no document mutation until the user actually drags).
- `FastResizeStart` is `destructive` (it transitions
  `DragState::None → DragState::Throttled(NodeResize)`, which on
  release writes through `set_node_aabb`).

The `is_destructive` flags are set on the `#[action(...)]` attribute;
the existing test at `keybinds/tests.rs:467-506` (destructive-set
exhaustiveness) catches drift.

### 3.8 Keybind defaults

New `KeybindConfig` fields with defaults:

```rust
// Existing field is renamed cancel_mode → exit_mode (per §3.3).
// Default keybind unchanged (Esc).
pub exit_mode: Vec<String>,

// NEW
pub enter_node_edit: Vec<String>,        // default: ["Enter"] (replaces edit_selection's Enter binding)
pub enter_section_edit: Vec<String>,     // default: ["Enter"] (in NodeEdit context)
pub enter_resize_mode: Vec<String>,      // default: ["r"]
pub fast_resize_start: Vec<ParametricBinding>, // default: ["Ctrl+RightDrag"]
```

The pre-existing `edit_selection: vec!["Enter"]` field is deleted —
`EditSelection` is renamed to `EnterNodeEdit` and its dispatch body
absorbs the EdgeLabel / PortalText branches that didn't fit
elsewhere. (Per CODE_CONVENTIONS §10 — no aliases.)

`edit_selection_clean` is similarly renamed to `enter_node_edit_clean`
with default `["Backspace"]` (unchanged binding).

### 3.9 Console verbs for mode transitions

Three new console verbs / subverbs land alongside the Actions:

- **`mode`** (new top-level verb)
  - `mode show` — print the current mode
  - `mode default` — exit any active mode (= `ExitMode`)
  - `mode reparent` — enter Reparent mode targeting current selection
  - `mode connect` — enter Connect mode
  - `mode resize` — enter Resize mode targeting current selection
  - `mode node-edit` — enter NodeEdit mode targeting current selection
  - `mode section-edit [section=N]` — enter SectionEdit mode

- **`node edit`** (new subverb on existing `node` verb)
  - Equivalent to `mode node-edit`. Provided for discoverability —
    a user who knows `node resize` will guess `node edit`.

- **`section edit [<idx>]`** (new subverb on existing `section` verb)
  - With selection on a node or section: enters NodeEdit if not
    already there, then enters SectionEdit on the named index (or the
    selection's section if no positional given).

Each new verb is a single dispatch arm calling the corresponding
`Action`. Existing Actions stay the entry point — verbs are thin.

### 3.10 What the user sees end-to-end

A guided tour to lock the mental model in:

1. User opens a map. `InteractionMode::Default`. Status bar empty.
2. Click on a multi-section node "Recipes". `SelectionState::Single("recipes")`.
   No section frames yet — we're in Default mode, the node is the
   unit. Resize handles **do not appear** (this is the change from
   today).
3. Press `Enter` (or type `node edit` in the console).
   `InteractionMode::NodeEdit { node_id: "recipes" }`. Section frames
   appear around each section. Inactive nodes dim. Status bar reads
   `editing: recipes — section [0 of 3]`.
4. Click on the third section. `SelectionState::Section(SectionSel { node_id: "recipes", section_idx: 2 })`.
   The third section's frame brightens. Status bar updates to `[2 of 3]`.
5. Press `Enter` again. `TextEditState::Open { node_id: "recipes", section_idx: 2, ... }`.
   Cosmic-text caret appears in the third section. Status bar reads
   `editing text: recipes[2]`. Type freely.
6. Press `Esc`. Editor closes (commit). Mode falls back to
   `NodeEdit { node_id: "recipes" }`. Caret gone, frame still bright.
7. Press `Esc` again. Mode falls back to `Default`. Section frames
   gone, dimmed nodes restore. `SelectionState::Single("recipes")`.
8. Press `r` (or click `mode resize`). `InteractionMode::Resize { target: Node("recipes") }`.
   8 resize anchors appear at the node's corners and edge midpoints.
   Body tints subtly. Status bar reads `resize: recipes — drag a corner or edge`.
9. Drag the SE handle. Standard `Throttled(NodeResize)` gesture; node
   grows. Release commits via `set_node_aabb`.
10. Press `Esc`. Mode → `Default`. Anchors gone.
11. (Alternative to steps 8–10:) Hold `Ctrl` + right-mouse-drag from
    anywhere on the node body. Anchor inferred from cursor quadrant;
    same `Throttled(NodeResize)` body. Release. No mode entered, no
    handles ever shown.

Every transition above corresponds to one `Action` invocation, every
one of which is reachable from console + keybind + (future) GUI.
## 4. Section UX redesign

This section depends on §3 (the mode system) being landed. Sections
4.1 through 4.6 each cite the file:line surface they modify.

### 4.1 Click routing change — single-click on multi-section node sets `Single`

**Today** (`/home/user/mandala/src/application/app/click.rs:62-71`):

```rust
(Some(id), false) => {
    if let Some(section_idx) = hit_section {
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx,
        });
    } else {
        doc.selection = SelectionState::Single(id.clone());
    }
}
```

**After** — click routing consults `InteractionMode`:

```rust
(Some(id), false) => {
    let route_to_section = match interaction_mode {
        InteractionMode::NodeEdit { node_id } if node_id == id => true,
        _ => false,
    };
    if route_to_section {
        if let Some(section_idx) = hit_section {
            doc.selection = SelectionState::Section(SectionSel {
                node_id: id.clone(),
                section_idx,
            });
        } else {
            // NodeEdit-mode click on the node's own chrome
            // (between sections, in border padding): reset to
            // Single but keep the mode. Same shape as a click on
            // empty canvas inside Default — re-establishes "the
            // node is the active mini-canvas" target without
            // committing to a specific section.
            doc.selection = SelectionState::Single(id.clone());
        }
    } else {
        doc.selection = SelectionState::Single(id.clone());
    }
}
```

The shift+click path (`click.rs:72-150`) gets the same gate — section
toggling into `MultiSection` requires `NodeEdit` mode on the same
node. Outside `NodeEdit`, shift+click on a section folds to
shift+click on the owning node (toggling whole-node `Multi` selection
the way it does today for whole-node clicks).

The `hit_test_target` fold for single-section nodes
(`hit_test.rs:130-138`) is unchanged — it still returns
`HitTarget::NodeContainer` for `sections.len() == 1`. The new
mode-gate above only fires when `hit_section.is_some()`, which
single-section nodes never produce. So legacy single-section maps
keep their exact current click behavior. (Principle §1.7.)

The section-drag promotion (`event_cursor_moved.rs:417-454`) gets
the same gate: `MovingSection` only promotes inside `NodeEdit`. In
`Default` mode, dragging on a section behaves identically to
dragging on the node's body — moves the whole node.

**Click outside the active node exits NodeEdit** — handled in
`event_mouse_click.rs` Released branch via a new helper
`maybe_exit_node_edit_on_outside_click(interaction_mode, hit, doc)`.
The exit dispatches `Action::ExitMode` for parity with the keybind
path. Outside-click is determined by `point_in_node_aabb` (which is
already overflow-aware and will count the active node's
overflowing-section territory as "inside").

### 4.2 Single-section nodes short-circuit `EnterNodeEdit`

Per §3.4, `Action::EnterNodeEdit` checks `node.sections.len()`:

- `len() == 1`: open `TextEditState::Open { node_id, section_idx: 0, ... }`
  AND set `InteractionMode::NodeEdit { node_id }` in one pass. The
  user experiences "Enter on a node opens the editor", matching
  today's UX for legacy single-section maps.
- `len() > 1`: set `InteractionMode::NodeEdit { node_id }`. The user
  must then click a section + press Enter again, or type
  `section edit <idx>`.

This is the only conditional in the mode-transition layer. It's
preserved everywhere (`Action::EnterNodeEdit` arm, `node edit` console
verb, `mode node-edit` console verb) — single source via the helper
`apply_enter_node_edit`.

### 4.3 NodeEdit mode rendering

Three additions to the scene-rebuild path
(`/home/user/mandala/src/application/app/scene_rebuild.rs`):

1. **Section frames.** A new pass `build_section_frames(map, mode, ...)`
   in `lib/baumhard/src/mindmap/scene_builder/section_frame.rs`
   (new file). Emits one `SectionFrameElement` per `MindSection` of
   the active node when `mode == InteractionMode::NodeEdit { ... }`.
   Style: thin glyph border in `SELECTED_EDGE_COLOR`, drawn as a
   single `GlyphArea` per section using box-drawing glyphs. The
   element family registers in `SceneSelectionContext`'s
   `RenderScene` struct as a new `Vec<SectionFrameElement>` field;
   the renderer treats it as its own buffer family (parallel to
   `section_resize_handles`).

2. **Inactive-node dimming.** A new field
   `dim_other_nodes_for: Option<&str>` on `SceneSelectionContext`.
   When `Some(node_id)`, the per-node text + chrome rendering passes
   apply a 50% alpha multiplier to every node whose id ≠ the active
   one. Implementation: in
   `lib/baumhard/src/mindmap/scene_builder/node_pass.rs`, the
   text-region resolution loop (`:135-169`) tints the resolved color
   when the node is inactive. The dim is uniform across text /
   border / connections — so visually the active node is
   "highlighted" (full alpha) and the rest of the canvas falls back.

3. **Status bar.** A new `mode_status_line: Option<GlyphArea>` field
   on `AppScene` (the host, not the per-tree scene), painted in the
   overlay layer. The string is computed in `scene_rebuild.rs` per
   frame from `(mode, selection, doc)`. Uses the same overlay
   pattern as the FPS overlay in `application/scene_host.rs`.

**Cost note.** Per CODE_CONVENTIONS §4 (mobile budget), section
frames render only when in NodeEdit mode and only for the active
node — at most a small constant (≤ 16 sections in practice) of
extra `GlyphArea`s per frame. Inactive-node dimming is one
multiplication per text-element resolution; benchmarked-equivalent
to the existing `apply_tree_highlights` walk.

### 4.4 SectionEdit visual: `Flag::Focused`

When `TextEditState::Open` activates, the editor's `apply_text_edit_to_tree`
also sets `Flag::Focused` on the active section-area's `GfxElement`
via a `DeltaGlyphArea::Flag` mutation. Closing the editor unsets it.
The renderer's section-frame pass reads the flag and renders the
focused section's frame at 1.5× thickness / 100% alpha (vs. 1× /
70% for unfocused sections in the same NodeEdit mode).

This wires `Flag::Focused` for the first time. There's no migration
cost — no current consumer to break.

### 4.5 Section console verb redesign

The current `section move <dx> <dy>` and `section resize <w> <h>`
syntax with positional args is replaced by a kv-style API consistent
with `color`, `font`, `zoom`. The selection model is also relaxed
(MultiSection fans out for `move`; `Single`-on-single-section-node
no longer requires `section=K`).

#### New surface

```
section move dx=<f64> dy=<f64> [section=<idx>]
section move x=<f64> y=<f64> [section=<idx>]      # absolute (NEW)
section resize w=<f64> h=<f64> [section=<idx>]
section resize fill [section=<idx>]                # alias for resize w=fill h=fill
section show [section=<idx>]                       # NEW: per-section info readout
section edit [<idx>]                               # NEW: enter SectionEdit
section text "<text>" [section=<idx>] [runs=preserve|clear]   # NEW: replace text
section add [at=<idx>] [text="<text>"]             # NEW: insert a new section
section delete [section=<idx>]                     # NEW: remove a section
section split [section=<idx>] [at=<grapheme>]      # NEW: split a section in two
```

`section move dx=... dy=...` is the delta-based mover (today's
positional `<dx> <dy>` form). `section move x=... y=...` is the
absolute setter (new — closes a gap noted in the section investigation,
§9.1.8).

`section resize fill` replaces the awkward `section resize none`
(`none` reads as "remove the section" rather than "make it
fill-parent"). The old `none` is **deleted** per CODE_CONVENTIONS §10
— users update their muscle memory.

#### Selection resolution (kv-style, consistent across verbs)

```
resolve_section_idx(args, doc) -> Result<(node_id, section_idx), String>:
  1. If args has `section=K`, K is the index. Owner is selection's
     primary_node_id (Single | Section | SectionRange) — error if
     selection is None / Multi / MultiSection / Edge.
  2. Else if selection is Section(s) | SectionRange { sel: s, .. }: (s.node_id, s.section_idx).
  3. Else if selection is Single(id) AND mindmap.nodes[id].sections.len() == 1: (id, 0).
     (Closes the §5.7 hostile error today: a single-section node implies section 0.)
  4. Else if selection is MultiSection(secs):
     - For `move dx=X dy=Y`: fan out — call set_section_offset for every entry. (§9.1.3)
     - For `resize` and others: error — single-target only.
  5. Else: error with verb-specific message.
```

#### Implementation skeleton

`section.rs` is rewritten to follow the `color.rs` template. Key
seams:

- Each subverb (`move`, `resize`, `show`, `edit`, `text`, `add`,
  `delete`, `split`) is a small `fn execute_section_<subverb>(args,
  eff)` that:
  - calls `helpers::collect_kvs_or_usage` for kv extraction.
  - calls `resolve_section_idx` (above).
  - calls one of the doc setters listed below.
  - finalises via `helpers::ApplyTally` + `applied_or_no_change`.

- Two new doc setters:
  - `MindMapDocument::add_section(node_id, at: Option<usize>, section: MindSection) -> Result<usize, String>`:
    inserts at `at.unwrap_or(node.sections.len())`. Validates AABB.
    Returns the new section's index. Push `EditNodeStyle` undo entry.
  - `MindMapDocument::delete_section(node_id, idx) -> Result<MindSection, String>`:
    removes the section at idx. Errors if `sections.len() == 1`
    (the model invariant — every renderable node has at least one
    section). Returns the removed section for undo restoration.
    Push `EditNodeStyle`.

- One new doc setter for split:
  - `MindMapDocument::split_section(node_id, idx, at_grapheme: Option<usize>) -> Result<usize, String>`:
    splits the section into two at the given grapheme position
    (defaulting to the end of the existing text — i.e. just clones
    the section and clears one half's text). Returns the index of
    the new sibling section. Push `EditNodeStyle`.

The atomic AABB setter `set_section_aabb` (already exists) becomes
reachable via console as `section move x=<f64> y=<f64> w=<f64> h=<f64> [section=<idx>]`
(combined move+resize). Validation rejects intermediate-state failures
the way `set_section_offset`+`set_section_size` would.

#### `section show`

Per the `border show` template. Outputs:

```
section[0] of node "0.1.2"
text:    "Hello world"
runs:    3 runs (1 bold, 0 italic, 0 underline, 1 hyperlink)
offset:  (10.0, 20.0)
size:    Some(120 × 40) [explicit pin]   # or "None [fill parent: 200 × 80]"
channel: Some(2)                          # or "None [→ index 0]"
bindings: 1 trigger (OnClick → switch-dark)
```

Format function in
`/home/user/mandala/src/application/console/commands/section/show.rs`
(new file). Routes through `OutputLine::plain` and
`OutputLine::in_font` for the text preview.

#### Tab completion improvements

- `section <TAB>`: surface `move | resize | show | edit | text | add | delete | split` plus the `section=K` kv key. (Today only `move | resize` show.)
- `section move <TAB>`: surface kv keys `dx=`, `dy=`, `x=`, `y=`, `section=` with hints.
- `section resize <TAB>`: kv keys `w=`, `h=`, `section=` plus the literal `fill`.
- `section edit <TAB>` and `section delete <TAB>` and `section show <TAB>`: positional integer; **value completion is selection-aware** — emit `0..node.sections.len()` for each candidate, with a per-row hint showing the section's preview text.
- `section section=<TAB>`: same selection-aware integer completion.

The completion logic lives in `console/commands/section/complete.rs`
(matching the existing `border/complete.rs` pattern, which the
section module is being upgraded to mimic).

#### `section text "..."`

Today, console paths can't change a section's text. The verb:

- Reads the text payload (`text=` kv with optional `runs=preserve|clear`
  — default `preserve`).
- Calls `MindMapDocument::set_section_text` (already exists at
  `nodes/section_text.rs:115`) when `runs=clear`, or
  `set_section_text_and_runs` with the existing runs preserved when
  `runs=preserve`. (`set_section_text_and_runs` exists at
  `nodes/section_text.rs:59` — its "preserve runs through a text
  rewrite" branch is what `runs=preserve` invokes.)

This closes a major gap (§9.8 in the section investigation): users
can now author multi-section nodes from the console.

#### `section add` and `section delete`

```
section add at=<idx> text="<text>"
```

Inserts a new section at index `at` (default end). The new section
defaults to `offset = (0, 0)`, `size = None` (fill-parent), `channel
= None` (→ idx), `text_runs = []`, `trigger_bindings = []`. The verb
dispatches the new `MindMapDocument::add_section` doc setter.

```
section delete [section=<idx>]
```

Removes the section at idx. Errors if it would leave the node with
zero sections.

#### Predicate / applicability

`section` becomes `applicable: node_or_section_selected` (a new
predicate added to `predicates.rs` — bundles `Single | Section |
SectionRange | MultiSection`). Edge-adjacent selections cause the
verb to be hidden in `help` and completion. (Closes §5.4 in the
console-patterns investigation — the "applicable: always but
runtime-rejects" anti-pattern.)

### 4.6 New Action variants for sections

To match the console verbs, the Action enum gains:

```rust
/// section move dx=<dx> dy=<dy> [section=<idx>] — delta nudge.
/// Target section per the selection-resolution rules in §4.5.
/// Replaces today's `SetSectionOffsetDelta`.
#[action(context = Document, wasm = Compatible)]
SetSectionOffsetDelta { dx: String, dy: String },   // unchanged

/// section move x=<x> y=<y> [section=<idx>] — absolute set. NEW.
#[action(context = Document, wasm = Compatible)]
SetSectionOffsetAbs { x: String, y: String },

/// section resize w=<w> h=<h> [section=<idx>] — explicit pin.
#[action(context = Document, wasm = Compatible)]
SetSectionSizeAbs { w: String, h: String },        // unchanged

/// section resize fill [section=<idx>] — flip to fill-parent.
#[action(context = Document, wasm = Compatible)]
SetSectionSizeFillParent,                           // unchanged (was already named correctly)

/// section text "<text>" [section=<idx>] [runs=preserve|clear]. NEW.
/// Carries the resolution mode as a string — `parse_runs_mode`
/// in dispatch. Destructive (rewrites text content).
#[action(context = Document, wasm = Compatible, destructive)]
SetSectionText { text: String, runs_mode: String },

/// section add [at=<idx>] [text="<text>"]. NEW. Destructive.
#[action(context = Document, wasm = Compatible, destructive)]
AddSection { at: String, text: String },

/// section delete [section=<idx>]. NEW. Destructive.
#[action(context = Document, wasm = Compatible, destructive)]
DeleteSection,

/// section split [at=<grapheme>] [section=<idx>]. NEW. Destructive.
#[action(context = Document, wasm = Compatible, destructive)]
SplitSection { at: String },
```

The dispatch arms route to the same setters the console verbs do.
Macro privilege gates: every `destructive` variant is User-tier-only
(or above) per the existing `SourceTier::allows_action` denylist.

### 4.7 Section visual hover affordance

While in `NodeEdit`, hovering the cursor over a section without
clicking sets a per-frame "hovered section" hint that the section-
frame pass renders at 1.2× brightness. Implementation: a new
`hovered_section: Option<(String, usize)>` field on `InitState`
(parallel to today's `hovered_node`), updated by `event_cursor_moved`
when the active mode is `NodeEdit`. Hovering exits when the cursor
leaves the section's AABB.

This is a small affordance, but it makes the "this is the section
you'd click on" behavior discoverable. Cost: one BVH descent
re-purposed (the same `descendant_at` path the click handler uses).

### 4.8 SelectionState lift on EnterSectionEdit / SectionRange

Today's `lift_anchor_to_section_range` (`text_edit/editor.rs:236-255`)
already lifts shift-select-during-edit to `SectionRange` on commit.
This is preserved verbatim. No changes needed — but the redesign
clarifies the user's mental model: SectionRange selection is the
"selection mode for sub-grapheme spans inside one section's text"
state, reached only through the editor's shift-select gesture.

### 4.9 Open question: should `Section` selection auto-imply NodeEdit?

The user's request says single-click on a node selects the whole
node. We've said: yes, in Default mode. And clicks on sections in
NodeEdit mode set `Section`. But what about: `Section` selection
established via a console verb (`select section node=N idx=M` if it
existed, or via `section show section=2`)?

**Decision:** No auto-mode. A `Section` selection without
`InteractionMode::NodeEdit { matching node_id }` is legal and means
"a section is selected for targeting (verbs apply to it), but we are
not in node-edit mode". Console verbs that target a single section
work fine; the canvas chrome is whatever Default mode shows.
Entering NodeEdit manually then composes — `Section` selection
inside NodeEdit lights up frames + status-bar.

Rationale: macros and scripted authoring need to set `Section`
selection without flipping a UI mode. Coupling the two makes that
impossible.

### 4.10 Migration of existing tests

Tests under `console/commands/section.rs:204-420` and
`document/tests_nodes.rs` (the section setters) require updates:

- The error message for the no-`section=` case on a `Single` selection
  becomes `"section: select a section or pass section=<idx>; or
  click into the section in NodeEdit mode"` (improved per §9.4 of
  the section investigation — the old "select a specific section
  (multi-section node)..." message claimed there was no path for
  single-section nodes, which is wrong).
- The MultiSection-rejection test now needs to assert fan-out
  behavior for `section move`, retaining rejection only for
  `section resize`.
- New tests added for: each new doc setter (`add_section`,
  `delete_section`, `split_section`); each new console subverb;
  the click-routing change (Default vs NodeEdit on multi-section
  nodes).

Test fixtures that today produce multi-section nodes via
`tests_common::pinned_two_section_node()` continue to work; new
fixtures added for the add/delete tests.
## 5. Border UX redesign

### 5.1 Goals

The current `border` verb has the right power surface — preset,
font, color, palette, padding, side patterns, corner glyphs — but
the wrong shape. The redesign keeps every capability the model
exposes and only changes the verb grammar, the completion vocabulary,
and the error / preview surfaces. It also fixes three latent bugs.

Specific goals (each maps to a `format/border-patterns.md` /
`format/schema.md` capability that the current verb already covers
or should cover):

1. Make the verb **discoverable**: typing `border ` and tabbing should
   surface a small grouped menu of high-level intents (preset, color,
   side, corner, palette, padding, show, reset), not a flat list of
   16 mixed positional verbs and kv keys.
2. Make `border on size=12` actually do what it says — fix the
   silent-drop bug.
3. Make the conflation between `style.frame_color` and
   `style.border.color` user-visible: one verb (`border color`) writes
   the override; the other (`color border=…`) writes the cascade
   default. `border show` lists both, labeled.
4. Add **live preview** for preset / color / side-pattern edits — the
   user sees the proposed border on the selected node before
   committing. This is the biggest UX leverage on top of the existing
   model.
5. Add a **canvas-default editing surface** (`canvas border …`).
   Today the model field exists (`canvas.default_border`) but is
   only authorable via raw JSON.
6. Make every preset choosable by a numeric or single-letter shortcut
   (`border preset=h` for `heavy`) and add `border preset=cycle` for
   "rotate to the next preset" — useful for keybind and macro driving.
7. Make per-side patterns templatable — surface common patterns as
   completion hints with literal previews (`+=##=+`, `###(*)###`,
   `─`, `═`).
8. Drop the silent auto-promotion of preset to `custom` when the user
   sets a side glyph — surface it as a typed message line (already
   partially done) but also offer an explicit `border preset=custom`
   path so authors who *want* per-side customisation can declare it.

### 5.2 New verb grammar — subverb-oriented

The new `border` verb is a small set of subverbs, each owning its
own kv keys and its own help text. The flat 16-key kv form is
**deleted** per CODE_CONVENTIONS §10 (no aliases, no two grammars).
Users update; the verb body is rewritten once.

```
border show [side=<top|bottom|left|right|all>] [verbose]
border on
border off
border toggle
border reset

border preset <name|cycle>           # name in light|heavy|double|rounded|custom; cycle picks the next
border preset=<name>                 # alias for the positional form (keep one kv form for keybinds)

border color <#hex|var(--name)|preset|reset>
border color=<value>                 # alias for keybinds

border palette <name|off> [field=<frame|background|text|title>]
border palette=<value> [field=<value>]   # alias

border padding <px>
border padding=<px>                  # alias

border font <family|off> [size=<pt>]
border font=<value> [size=<value>]   # alias

border side <top|bottom|left|right|all> <pattern>
border side <which> reset            # restore the side to the preset's default
border corner <tl|tr|bl|br|all> <glyph>
border corner <which> reset          # restore the corner to the preset's default

border preview <field>=<value> ...   # apply a transient preview, do NOT commit
border preview commit                # commit the current preview
border preview cancel                # discard the current preview
```

Notes on the grammar:

- **Each subverb either takes a positional value or a `=`-form.**
  The positional form is what users type (`border preset rounded`);
  the `=` form is what keybinds and macros use (`border preset=rounded`
  is a single token, easier to bind to a single combo).
- **`border side` and `border corner` are the only two-positional
  subverbs.** `border side top "###(*)###"` (or
  `border side=top pattern="###(*)###"`).
- **`border padding=8` is unambiguous** (no alternative fields named
  `padding`). The positional form `border padding 8` works too.
- **No more `border tl=`, `tr=`, `bl=`, `br=` shortcuts.** Use
  `border corner tl '+'` (single char) — three more characters but
  zero ambiguity. Tab completion for `border corner <TAB>` lists
  `tl | tr | bl | br | all`.
- **`border preview` is a new sub-mode** — see §5.6 below. Composable
  with every other subverb via the `border preview <field>=<value>`
  form; commits or cancels via the explicit subverbs.

### 5.3 Why this shape (and what it preserves)

The redesign preserves every capability `BorderConfigEdits`
(`/home/user/mandala/src/application/document/nodes/border.rs:32-58`)
exposes today:

| Edit | Old verb | New verb |
|---|---|---|
| `style.show_frame = true` | `border on` | `border on` (unchanged) |
| `style.show_frame = false` | `border off` | `border off` (unchanged) |
| toggle visibility | n/a | `border toggle` (NEW) |
| drop the override | `border reset` | `border reset` (unchanged) |
| `cfg.preset = "..."` | `border preset=heavy` | `border preset heavy` |
| cycle preset | n/a | `border preset cycle` (NEW) |
| `cfg.font = ...` | `border font=Inter` | `border font Inter` |
| `cfg.font_size_pt = ...` | `border size=14` | `border font size=14` (or `border font Inter size=14`) |
| `cfg.color = ...` | `border color=#fff` | `border color #fff` |
| `cfg.color_palette = ...` | `border palette=Coral` | `border palette Coral` |
| `cfg.color_palette_field = ...` | `border field=frame` | `border palette Coral field=frame` |
| `cfg.padding = ...` | `border padding=8` | `border padding 8` |
| `cfg.glyphs.top = pattern` | `border top="..."` | `border side top "..."` |
| `cfg.glyphs.top_left = '+'` | `border tl="+"` | `border corner tl '+'` |

`border show` gets a `side=` filter (so the user can ask just for
the four side patterns) and a `verbose` flag (which prints the
default cascade chain — per-node override, then canvas default,
then hardcoded floor).

### 5.4 Bugfixes baked in

Three latent bugs surfaced by the border investigation are fixed in
the redesign:

1. **`border on/off/reset` ignored kvs.** The new grammar separates
   subverbs from kv composition entirely — `border on` takes no
   arguments. To compound, the user runs them as separate verbs
   (`border on; border preset heavy; border padding 8`) or uses
   `border preview`. This makes the silent-drop impossible by
   construction.

2. **`color border=…` and `border color=…` write different fields.**
   `border show` (with `verbose`) now lists both:
   ```
   color (cascade):
     style.frame_color    = "#30b082"          # set via `color border=`
     style.border.color   = None [→ frame]     # set via `border color`
   ```
   The user sees the dual surface and can target either explicitly.
   The dispatch difference itself stays — neither verb changes
   behavior, only the readout does.

3. **`preset=custom` with no glyph fields was an awkward sentinel.**
   The new grammar makes it explicit: `border preset custom` enters
   custom-preset mode but does not change glyphs. To set glyphs,
   use `border side` / `border corner`. The hint message is replaced
   by a clear "next steps" line:
   ```
   border preset custom: ready for per-side glyphs.
   try: border side top "+=##=+"  or  border corner tl '+'
   ```
   The auto-promotion of `preset` to `custom` when a side/corner is
   set against `preset=heavy` is **removed** — instead, `border side`
   on a non-custom preset returns an error:
   ```
   border side top: cannot set side glyph against preset 'heavy'.
   run `border preset custom` first, then set the side.
   ```
   This is a deliberate UX shift away from "do what I mean" toward
   "tell me what I should have asked for". The auto-promote behavior
   was a recurring confusion (per investigation §7.1).

### 5.5 New idiomatic shape

The verb file becomes `console/commands/border/` with subverb
modules:

```
console/commands/border/
  mod.rs           # COMMAND, KEYS, registration, applicability
  show.rs          # border show
  preset.rs        # border preset (named, cycle, custom)
  color.rs         # border color
  palette.rs       # border palette + field
  padding.rs       # border padding
  font.rs          # border font + size
  side.rs          # border side
  corner.rs        # border corner
  preview.rs       # border preview / commit / cancel (§5.6)
  on_off.rs        # border on / off / toggle / reset
  complete.rs      # tab completion (refactored)
  tests/
    mod.rs
    each subverb has a focused test file
```

Each subverb file follows the canonical idiom from the
console-patterns audit:

1. Top-of-file: subverb-specific KEYS const (only the kvs this
   subverb accepts).
2. `pub(super) fn execute_<subverb>(args, eff) -> ExecResult` —
   uses `helpers::collect_kvs_or_usage`, validates, dispatches to
   the doc setter (today's `set_node_border_config(... edits)` with
   a single-field edit), uses `helpers::ApplyTally::finalize` for
   the per-target tally (instead of the bespoke `BorderEditOutcome`
   tally).
3. Tests: at least one happy-path, one error-path, one undo round-
   trip. Per the section investigation's recommended idioms (§9
   bullet 6).

Applicability becomes
```rust
applicable: node_or_section_selected,    // new predicate
```
(same predicate the section verb uses — see §4.5). Edge-adjacent
selections hide the verb in `help` and completion.

The bespoke `BorderEditOutcome` is **removed**; the auto-promote
warning becomes a typed `Outcome::Lines(Vec<OutputLine>)` line
emitted by the `preset` subverb when the user opts into a custom
preset. The preset-promotion logic in
`document/nodes/border.rs:286-293` (which silently flipped preset to
custom when a side glyph was set) is **deleted** — the new error
message in §5.4.3 catches this case at the verb layer instead.

### 5.6 Live preview (`border preview`) — *shipped*

**Status: implemented across all four border surfaces** (per-node,
per-section, two canvas defaults). Users frequently want to see
what a preset / color / pattern looks like before committing —
preview stages the edit on a transient slot, the renderer
substitutes the staged style, and the user terminates with
`commit` (writes through the matching committing setter) or
`cancel` (discards).

```
border preview preset=heavy color=#ff8800   # per-node
border preview commit
border preview cancel

section frame preview top="###(*)###"        # per-section
canvas border preview palette=rainbow        # canvas default
canvas section-frame focused preview preset=double
```

As-shipped implementation diverges from the original sketch in
two places — both flagged during the review-fix passes:

- **Target shape**: `BorderPreview { target: BorderPreviewTarget,
  edits: BorderConfigEdits, selection_snapshot: SelectionState }`
  where `BorderPreviewTarget` is a 5-variant enum
  (`Nodes(Vec<String>)` / `Sections(Vec<(String, usize)>)` /
  `CanvasDefault` / `CanvasSectionFrame` /
  `CanvasSectionFrameFocused`). The original `node_ids:
  Vec<String>` shape was per-node-only and didn't fit the four
  surfaces.
- **Selection-drift posture**: lazy defer-clear, not eager
  cancel-on-change. The renderer treats a drift-detected preview
  as inactive; the actual slot clear happens at the next `set_*`
  / `commit_*` / `cancel_*` call. Hooking the ~25 sites that
  write `MindMapDocument.selection` would be fragile, and an
  eager cancel on every tab key was annoying in practice.
- **Scene-builder hook**: extends `build_scene_with_cache` with
  `border_preview: Option<BorderPreview<'a>>` (peer of the
  existing `edge_color_preview` / `portal_color_preview`); not a
  field on `SceneSelectionContext`. Single chokepoint at
  `assemble_scene_overrides` constructs the borrowed view.
- **Esc behavior**: chained inside `Action::ExitMode`'s body —
  `cancel_border_preview()` runs first, short-circuits when a
  preview was canceled, and otherwise falls through to the
  normal mode-clear path. The keybind resolver maps
  `(context, key) → Action` deterministically and can't fall
  through, so `cancel_border_preview` ships unbound by default
  (the chain is the user-visible behavior).
- **Implicit cancel on committing edits**: any of the four
  committing setters clears the preview as their first line, so
  a non-preview edit always wins.

Programmatic surface: `Action::SetBorderPreview { target_kind:
BorderPreviewTargetKind, field, value }` (single kv per Action;
multi-kv preview stays console-only),
`Action::CommitBorderPreview`, `Action::CancelBorderPreview`.

The preview slot does **not** persist through document load/save
cycles (runtime ephemeral state, not serialized).

### 5.7 Canvas-default editing

Today `canvas.default_border` is editable only via raw JSON
(investigation §1.9). The redesign adds:

```
canvas border show
canvas border preset <name>
canvas border color <value>
canvas border palette <name> [field=<value>]
canvas border padding <px>
canvas border font <family> [size=<pt>]
canvas border side <which> <pattern>
canvas border corner <which> <glyph>
canvas border reset             # drop canvas default entirely
```

These mirror the per-node `border` verbs but operate on
`canvas.default_border` (which becomes `Some` on the first
`canvas border <field>` write, and is dropped to `None` on
`canvas border reset`).

A new doc setter `MindMapDocument::set_canvas_default_border(edits: BorderConfigEdits) -> bool`
mirrors `set_node_border_config` shape, pushing an `EditCanvasStyle`
undo entry (a new variant — there's no existing undo for canvas
state, but it's a small addition).

The `canvas` verb is otherwise unchanged today. This is a small
extension. The canvas verb is registered in
`console/commands/mod.rs:68-87` (currently absent — it's a new
top-level verb).

### 5.8 New Action variants

```rust
/// Mirror border preset <name> on selection. NEW.
#[action(context = Document, wasm = Compatible)]
SetBorderPreset(String),

/// Cycle preset (light → heavy → double → rounded → custom → light).
#[action(context = Document, wasm = Compatible)]
CycleBorderPreset,

/// Mirror border color <value>.
#[action(context = Document, wasm = Compatible)]
SetBorderColor(String),

/// Mirror border padding <px>.
#[action(context = Document, wasm = Compatible)]
SetBorderPadding(String),

/// Mirror border palette <name> [field=...].
#[action(context = Document, wasm = Compatible)]
SetBorderPalette { palette: String, field: String },

/// Mirror border font <family> [size=...].
#[action(context = Document, wasm = Compatible)]
SetBorderFont { family: String, size_pt: String },

/// Mirror border side <which> <pattern>.
#[action(context = Document, wasm = Compatible)]
SetBorderSide { side: String, pattern: String },

/// Mirror border corner <which> <glyph>.
#[action(context = Document, wasm = Compatible)]
SetBorderCorner { corner: String, glyph: String },

/// Toggle visibility (one Action, simpler than separate on / off).
#[action(context = Document, wasm = Compatible)]
ToggleBorderVisible,

/// Set / clear the live preview state. Carries the kvs as a
/// pre-parsed BorderConfigEdits — payload is a String key=value
/// list to keep Hash + Eq for the Action enum.
#[action(context = Document, wasm = Compatible)]
SetBorderPreview { kvs: String },

#[action(context = Document, wasm = Compatible)]
CommitBorderPreview,

#[action(context = Document, wasm = Compatible)]
CancelBorderPreview,
```

The existing `Action::SetBorderField { field, value }` is
**deleted** — its callers migrate to the more specific actions
above. The deletion is per CODE_CONVENTIONS §10 (no aliases).

### 5.9 Completion improvements

Per `border` subverb:

- `border <TAB>` (token 0): `show | on | off | toggle | reset | preset | color | palette | padding | font | side | corner | preview` — with each row carrying a one-line hint.
- `border preset <TAB>`: `light | heavy | double | rounded | custom | cycle`.
- `border color <TAB>`: hex prompt (no completions) but the
  per-key hint reads "use #rrggbb, var(--name), accent, edge, fg, or
  reset". Plus `accent | edge | fg | reset` as completions (the
  preset color names from `commands/color.rs`).
- `border palette <TAB>`: every palette key in `doc.mindmap.palettes`,
  plus `off`.
- `border palette ...  field=<TAB>`: `frame | background | text | title`.
- `border padding <TAB>`: no completions (numeric).
- `border font <TAB>`: every font family from `loaded_families_iter`,
  rendered in their own face (the per-row `font_family` tag is
  already in place at `border/complete.rs:106-117`).
- `border side <TAB>`: `top | bottom | left | right | all`.
- `border side top <TAB>`: a small set of common pattern templates,
  each rendered as the literal pattern in the border font:
  ```
  ─                    # solid line (atomic single)
  +=##=+               # atomic compound
  ###(*)###            # prefix+fill+suffix
  ╔(═)╗                # double-line with stretch
  =(=)=                # all-fill
  reset                # restore to preset default
  ```
  These are *templates* — the user picks one and edits the literal.
  Renders use `OutputLine::in_font` so the preview is in the border
  font.
- `border corner <TAB>`: `tl | tr | bl | br | all`.
- `border corner tl <TAB>`: a small set of common corner glyphs:
  ```
  ┌  ┏  ╔  ╭  +  *  ●  ◆  ◇  ▲  ▼  ◀  ▶
  reset
  ```
- `border preview <TAB>`: same vocabulary as `border` token-0,
  prefixed with `commit | cancel` for ending preview.

### 5.10 `border show` example output (after redesign)

```
visible: on  (toggle: `border off`)
preset:  heavy  (cycle: `border preset cycle`)
font:    Liberation Mono (12 pt)  (override: `border font ...`)
color (cascade):
  style.frame_color   = "#30b082"          # set via `color border=`
  style.border.color  = None [→ frame]     # set via `border color`
palette: (none)
padding: 4 px
size:    240 × 60 px (24 cluster cols, 4 rows)
top:     ━━━━━━━━━━━━━━━━━━━━━━━━
bottom:  ━━━━━━━━━━━━━━━━━━━━━━━━
left:    ┃   ┃   ┃   ┃
right:   ┃   ┃   ┃   ┃
corners: tl=┏  tr=┓  bl=┗  br=┛
```

(Side rows render in the border font, corner glyphs render in the
border font — same OutputLine::in_font path as today.)

### 5.11 Migration of existing tests

Tests under `console/commands/border/tests.rs` (23 tests today) are
rewritten to target the new subverbs. Most existing assertions
translate one-for-one:

- `border_on_then_off_toggles_show_frame` → `border_toggle_*` plus
  the explicit on/off variants.
- `border_preset_writes_field` → splits into per-preset name tests.
- `border_top_pattern_parse_error_surfaces_with_prefix` → moves to
  `side.rs`'s test file.
- `border_palette_records_palette_name` → moves to `palette.rs`'s
  test file.

New tests added:

- `border_preset_cycle_advances_through_table`.
- `border_preview_does_not_write_model`.
- `border_preview_commit_writes_then_clears`.
- `border_preview_cancel_clears_without_writing`.
- `border_preview_drops_on_selection_change`.
- `border_side_against_non_custom_preset_errors_with_pointer`
  (replaces today's auto-promote success).
- `canvas_border_preset_writes_canvas_default`.

Per CODE_CONVENTIONS §11 (extensive testing).

### 5.12 Documentation update

`format/border-patterns.md` adds a "Console verb" section covering
the new grammar (replacing today's small "Console verb" section
that lists the flat-kv form). The side-pattern grammar itself is
unchanged — only the verb that consumes it changes.

### 5.13 Out of scope for this batch

- Changing the underlying `BorderConfigEdits` shape (the model-side
  bundle stays as-is).
- Changing the side-pattern parser (`SidePattern::parse`).
- Adding new presets to the table (`heavy`, `light`, `double`,
  `rounded`, `custom` are the universe).
- Adding shape-aware borders for non-rectangular nodes
  (`format/enums.md:54-57` notes the silent-drop on ellipses; that
  visual fix is a renderer-level concern outside this UX overhaul).
## 6. Resize UX redesign

### 6.1 Goals

The single root cause of "we accidentally resize when we wanted to
move" is: **resize handles auto-emit on selection, and the
threshold-cross priority chain ranks handle hits above body hits**.
The fix is to gate handle emission on an explicit mode, and to
introduce a separate fast-resize path that doesn't need handles at
all.

Specific goals:

1. **Stop emitting resize handles on selection.** A `Single(node)` or
   `Section(s)` selection in `Default` mode shows zero anchors.
2. **Add an explicit Resize mode** with auto-emitted anchors. Entered
   via `Action::EnterResizeMode` (default keybind `r`), via
   `mode resize` console, or via a future GUI button.
3. **Add a fast-resize gesture**: hold `Ctrl + RightMouse` and drag
   anywhere on a node body to resize it. No anchors shown. Direction
   inferred from cursor quadrant relative to the node center. This is
   the Hyprland model — discoverable through keybinding hints, and
   fast enough that experienced users never bother with anchor mode.
4. **Reuse the existing throttled-drag plumbing**: both new
   mechanisms produce `DragState::Throttled(NodeResize)` /
   `Throttled(SectionResize)` and the same release-commit through
   `set_node_aabb` / `set_section_aabb`. Zero new mutation paths.
5. **Touch parity**: a long-press on a selected node enters Resize
   mode (anchor-driven), and a two-finger pinch-and-drag on a node
   triggers fast-resize.

### 6.2 Mechanism 1: Explicit Resize mode

#### State and entry

`InteractionMode::Resize { target: ResizeTarget }` (defined in §3).
Targets:
- `ResizeTarget::Node(node_id)` — fed from `SelectionState::Single`.
- `ResizeTarget::Section { node_id, section_idx }` — fed from
  `SelectionState::Section { ... }` or `SectionRange { sel: ..., .. }`.

Entry: `Action::EnterResizeMode` (defined in §3.3). Resolves
target from selection per §3.3 (see the doc-comment on the variant).
On `MultiSection` / `Multi` / `None` / edge variants → no-op + log.

Default keybind: `r` (single key, no modifier).

Console: `mode resize`. Or sugar via `node resize-mode` /
`section resize-mode` (both are aliases for `mode resize` after
verifying the selection matches the target type).

#### Chrome

- Resize handles render on the target — exactly today's 8-handle
  `□` glyph layout from `node_resize_handle.rs` and
  `section_resize_handle.rs`. Now gated on
  `interaction_mode.resize_handle_node()` /
  `interaction_mode.resize_handle_section()` instead of the
  selection-driven gate at `document/mod.rs:520-523`.
- Body of the targeted element gets a 10% cyan tint (per §3.5) so
  the user sees the active target distinctly from the rest.
- Status bar reads `resize: <target> — drag a corner or edge`.

#### Wiring change

In `/home/user/mandala/src/application/document/mod.rs:511-523`,
the `selected_section` and `selected_node_for_resize` arguments fed
into `SceneSelectionContext` are computed from `interaction_mode`,
**not** from `selection`:

```rust
let selected_section = interaction_mode.resize_handle_section();
let selected_node_for_resize = interaction_mode.resize_handle_node();
```

This is the single line change that fixes the auto-resize problem.
Everything else flows from it.

#### Hit-test in Resize mode

When `InteractionMode::Resize`, the press handler in
`event_mouse_click.rs:165-377` checks for handle hits **only on the
targeted element**. Today, handle hits are checked for any
`Single`-selected node and any `Some`-sized selected section
(`event_mouse_click.rs:298-345`). After the change:

- `hit_section_resize_handle` is computed only when
  `mode.resize_handle_section() == Some(target)`.
- `hit_node_resize_handle` is computed only when
  `mode.resize_handle_node() == Some(target_node)`.

The threshold-cross priority chain in `event_cursor_moved.rs:182-538`
is unchanged — the handle hits already take precedence over the body
when present, and that's the right behavior inside Resize mode
(every press in Resize mode IS a resize). Outside Resize mode, the
hits are never populated, so the priority chain never sees them.

#### Click outside the body in Resize mode

Click on empty canvas (outside the target's overflow-aware AABB)
exits Resize mode and returns to `Default`. Pre-fix the user was
trapped in implicit-mode-via-selection; the explicit mode makes the
exit affordance discoverable.

#### Esc behavior

`Action::ExitMode` (Esc default) drops the Resize mode. Selection
is preserved (the user wanted "resize this node", not "deselect").

#### Commit

Identical to today — `Throttled(NodeResize)` and
`Throttled(SectionResize)` release commits via `set_node_aabb` /
`set_section_aabb`. The mode itself doesn't reset on commit; the
user can drag multiple handles in succession without leaving Resize
mode. Only Esc / outside-click / `ExitMode` exits.

#### Multi-target ergonomics

When `Multi(ids)` is the selection, `Action::EnterResizeMode` warns
and no-ops:
```
resize: cannot enter resize mode on a multi-node selection.
        select a single node or section.
```
A future "uniform resize across N selected nodes" gesture is a
seam, not in scope.

### 6.3 Mechanism 2: Fast resize via `Ctrl+RightDrag`

#### Gesture

The user holds `Ctrl` and presses-and-drags the right mouse button
anywhere on a node body. The drag direction picks an anchor:

- Cursor in the **NW** quadrant of the AABB → anchor is `NW` (drag
  north-west grows east-south, drag south-east shrinks).
- Cursor in the **NE** quadrant → anchor is `NE`.
- Cursor in the **SW** quadrant → anchor is `SW`.
- Cursor in the **SE** quadrant (default for cursor at center) →
  anchor is `SE`.

Edge handles (N, E, S, W) are **not** auto-picked by quadrant — the
quadrant-based inference always picks a corner anchor. This is
deliberate: edge-only resize is a finer-grained operation that
needs the explicit anchor mode. Users who want only-X or only-Y
resize use Resize mode (§6.2) and target an edge handle directly.

Quadrant determined at press time, not continuously — once the
gesture is active, the chosen anchor sticks for the duration of the
drag (matching the handle-driven gesture semantics).

#### Mouse gesture variants

Add to `/home/user/mandala/src/application/keybinds/bind.rs:44-66`:

```rust
pub enum MouseGesture {
    LeftDrag,        // unchanged
    DoubleClick,     // unchanged
    MiddleClick,     // unchanged
    WheelUp,         // unchanged
    WheelDown,       // unchanged

    /// Single right-button press (no movement past threshold).
    /// Reintroduced — was removed for being half-feature; now has
    /// a real dispatch site.
    #[strum(serialize = "rightclick")]
    RightClick,

    /// Right-button held + cursor movement past the drag threshold,
    /// continuous gesture. Dispatched from event_cursor_moved when
    /// the press's pending state is "right-button held on node body".
    #[strum(serialize = "rightdrag")]
    RightDrag,
}
```

Both new variants get the same `pascal_form()` arm and `key_name()`
mapping as the existing five. The strum derive picks them up.

#### Default keybinds

```rust
fast_resize_start: vec![ParametricBinding {
    combo: "Ctrl+RightDrag".into(),
    args: vec![],
}],
```

Modifier-fallback (`action_for_gesture` in `resolved.rs:73-81`)
ensures `Ctrl+RightDrag` resolves to `Action::FastResizeStart` if
that exact binding exists, falling back to bare `RightDrag` if
not. So a user who unbinds `Ctrl+RightDrag` and binds bare
`RightDrag` gets fast-resize on plain right-drag, no modifier
required. (This is the standard pattern.)

`RightClick` defaults to `vec![]` (unbound). It's defined so that
users / future GUIs can bind context menus or other actions to it
without us needing to lift it into a half-feature later.

#### Implementation in mouse handlers

`event_mouse_click.rs:867`'s `_ => {}` catch-all is replaced with
explicit `MouseButton::Right` arms:

```rust
MouseButton::Right => {
    if state == ElementState::Pressed {
        // Compute hit, stash into DragState::Pending with a
        // new variant `PendingRight` (or extend the existing
        // Pending with a `is_right: bool` flag — pick the cleaner
        // option during implementation). The press-time hit covers
        // the body of any node or section; resize-handle hits do
        // NOT take precedence here (right-mouse is for fast-resize,
        // not for handle drag).
        ...
    } else {
        // Released: handle right-click via action_for_gesture(RightClick),
        // or finalize a Throttled(NodeResize)/Throttled(SectionResize)
        // gesture as today's left-button release does.
        ...
    }
}
```

Threshold-cross in `event_cursor_moved.rs` learns a new branch:
when `DragState::Pending` was a right-button press and the cursor
moves past the 5px threshold, dispatch
`Action::FastResizeStart` with the press-time hit as `DispatchHit`.
The Action's body computes the cursor quadrant relative to the AABB
center, picks the anchor, and transitions
`DragState::None → DragState::Throttled(NodeResize | SectionResize)`
with the chosen `ResizeHandleSide`.

Subsequent cursor moves drive the per-frame drain exactly as
today's handle-driven gesture does. Release commits the same way.

#### Cross-target

Resolution rules:

- Hit lands on a node body, not on a section: target is the node.
- Hit lands on a section body in a multi-section node: target is
  the section. (Single-section nodes always fold to NodeContainer
  per the hit-test rule, so this only fires for multi-section
  nodes.)
- Hit lands on empty canvas: log + no-op (right-mouse-drag on empty
  canvas is reserved — could be panning in a future binding).

The fold matches the existing left-click hit-test fold, so the
fast-resize gesture targets exactly what a user "would have
selected" with a left click in the same spot.

### 6.4 Anchor inference math

```rust
/// Pick a corner anchor based on cursor position within the AABB.
/// `aabb_center = (pos + size/2)`. Returns one of NW, NE, SW, SE.
pub fn infer_resize_anchor(cursor_canvas: Vec2, aabb_pos: Vec2, aabb_size: Vec2) -> ResizeHandleSide {
    let center = aabb_pos + aabb_size * 0.5;
    let east = cursor_canvas.x >= center.x;
    let south = cursor_canvas.y >= center.y;
    match (east, south) {
        (false, false) => ResizeHandleSide::NW,
        (true,  false) => ResizeHandleSide::NE,
        (false, true)  => ResizeHandleSide::SW,
        (true,  true)  => ResizeHandleSide::SE,
    }
}
```

Lives in `lib/baumhard/src/mindmap/scene_builder/section_resize_handle.rs`
next to `ResizeHandleSide::axis_factors` and `resolve_aabb` —
cohesive with the existing resize math. Tested as a pure function
(no GPU, no state).

### 6.5 Visual feedback in fast-resize

Even though no anchors render, the user gets feedback:

- The cursor changes to a `move` / `diagonal-resize` icon on the
  pressed node when the gesture starts. This uses the existing
  `cursor_is_hand` flag pattern at `input_context.rs:74-76`,
  generalised to `cursor_icon: CursorIconHint` with variants `Hand`,
  `ResizeNS`, `ResizeEW`, `ResizeNESW`, `ResizeNWSE`. The cursor
  variant is set by `event_cursor_moved` based on the active
  gesture / quadrant.
- Status bar transiently shows `resizing: <target>`.
- After release, the cursor reverts; status bar clears.

### 6.6 Touch / mobile

Touch is a peer (per §1.8). Two new gesture variants on
`MouseGesture` (despite the name — they live alongside mouse
gestures because they participate in the same keybind dispatch):

```rust
/// Touch: long-press (≥ 350ms) on a node, no movement.
#[strum(serialize = "longpress")]
LongPress,

/// Touch: two-finger drag (pinch + translate) on a node.
#[strum(serialize = "twofingerdrag")]
TwoFingerDrag,
```

Default keybinds:

```rust
enter_resize_mode: vec!["r".into(), "LongPress".into()],
fast_resize_start: vec![
    ParametricBinding { combo: "Ctrl+RightDrag".into(), args: vec![] },
    ParametricBinding { combo: "TwoFingerDrag".into(), args: vec![] },
],
```

A long-press on a node enters Resize mode; a two-finger pinch + drag
is fast-resize. The pinch direction (delta between the two fingers)
serves the same role as cursor quadrant for anchor inference — the
midpoint between the fingers is the cursor, the spread of the
fingers is the size delta. This requires touch handling to be
plumbed; per the action-investigation report, native and WASM both
have stub paths for `WindowEvent::Touch` but no real recognizer. The
recognizer lands in this batch (sized accordingly — this is the
single biggest piece of new code in the plan).

Implementation: a new `TouchGestureRecognizer` state machine in
`src/application/app/touch_gesture.rs` (new module, native + WASM)
that consumes `WindowEvent::Touch` events, tracks finger positions,
and emits `MouseGesture::*` synthetic events into the existing
mouse-input pipeline. This subsumes the half-wired touch
dispatch path the action investigation flagged as aspirational.

The recognizer has its own internal state machine separate from
`DragState` — it observes raw touch events, identifies gestures,
emits synthetic `MouseInput` / `CursorMoved` events. The existing
mouse dispatch then sees them as it would mouse events. Single source
of truth for "what gesture happened" stays in the keybind dispatch
layer.

### 6.7 Console verbs

```
node resize w=<f64> h=<f64>          # absolute setter (today, unchanged)
node resize-mode                     # enter Resize mode targeting selected node
node fit                             # shrink to text floor (today, unchanged)

section resize w=<f64> h=<f64> [section=<idx>]   # absolute (per §4.5)
section resize fill [section=<idx>]              # fill-parent (per §4.5)
section resize-mode [section=<idx>]              # enter Resize mode targeting section

mode resize                          # generic — picks node or section from selection
```

`node resize-mode` and `section resize-mode` are sugar for `mode
resize` with a selection check. They're convenient for keybinding
("go straight to resize mode for the section, even if my selection
is a node — pull up section 0 if no section is selected").

### 6.8 Removing the today-default behavior

The single change that removes today's auto-anchor-on-selection
behavior is in
`/home/user/mandala/src/application/document/mod.rs:520-523`:

```rust
// BEFORE (today):
let selected_node_for_resize = match &self.selection {
    SelectionState::Single(id) => Some(id.as_str()),
    _ => None,
};
let selected_section = self.selection.selected_section()
    .map(|s| (s.node_id.as_str(), s.section_idx));

// AFTER:
let selected_node_for_resize = interaction_mode.resize_handle_node();
let selected_section = interaction_mode.resize_handle_section();
```

That is the diff. Everything else in the resize subsystem stays
behaviorally identical — handles render the same way, hit-test
the same way, drag the same way, commit the same way; only the
predicate that decides "should the handles render" changes.

For the press-handler at `event_mouse_click.rs:309-345`, the gating
becomes mode-driven too:

```rust
// BEFORE:
let hit_section_resize_handle = match ctx.document.as_ref() {
    Some(doc) => match doc.selection.selected_section() {
        Some(s) => doc.hit_test_section_resize_handle(...).map(|side| ...),
        None => None,
    },
    None => None,
};

// AFTER:
let hit_section_resize_handle = match ctx.interaction_mode.resize_handle_section() {
    Some((node_id, section_idx)) => {
        let tol = HANDLE_HIT_TOLERANCE_PX * ctx.renderer.canvas_per_pixel();
        crate::application::document::hit_test_section_resize_handle(
            &doc.mindmap, canvas_pos, node_id, section_idx, tol,
        ).map(|side| (node_id.to_string(), section_idx, side))
    }
    None => None,
};
```

Same shape; selection-driven gate replaced by mode-driven gate.

### 6.9 Tolerance / aim

A separate concern flagged by the resize investigation: even with
explicit Resize mode, the 12px hit tolerance is large relative to
the visual handle size and the corner positions sit *at* the AABB
edge. The redesign adds two small fixes:

1. **Reduce `HANDLE_HIT_TOLERANCE_PX` from 12 to 8.** Inside Resize
   mode the user has fewer false positives (no body-vs-handle
   ambiguity to disambiguate); a tighter tolerance is fine and the
   handles are clickable enough at 8px.
2. **Inset corner handles by 2px.** Currently corner handles sit at
   the exact AABB corner; insetting them by 2px puts them slightly
   inside the body, making "click on the corner" unambiguously a
   handle hit. The 2px is configurable via a new
   `RESIZE_HANDLE_INSET_PX: f32 = 2.0` constant in
   `section_resize_handle.rs`.

These are minor follow-ups, called out for completeness — they
sit alongside the mode-driven gate but aren't strictly required to
fix the auto-resize bug.

### 6.10 Macro privilege

`Action::EnterResizeMode` is non-destructive (mode flip, no document
mutation). All tiers.

`Action::FastResizeStart` is destructive (it'll commit through
`set_node_aabb` on release). User-tier-only.

The split mirrors the EnterNodeEdit / EnterSectionEdit split in §3.7.

### 6.11 Tests

New tests (per CODE_CONVENTIONS §11):

- `enter_resize_mode_with_single_node_targets_node` —
  `Single(node)` selection; `EnterResizeMode` sets
  `InteractionMode::Resize { target: Node }`.
- `enter_resize_mode_with_some_section_targets_section`.
- `enter_resize_mode_with_none_section_logs_and_noops`.
- `enter_resize_mode_with_multi_node_logs_and_noops`.
- `resize_mode_emits_handles_on_target_only` — scene-builder pass
  with `Resize { Node }` mode shows 8 handles on that node, none on
  other selected nodes if any.
- `default_mode_emits_zero_handles` — `Default` mode + `Single`
  selection produces no handles.
- `infer_resize_anchor_picks_correct_quadrant` — pure function test
  for the four quadrants.
- `fast_resize_start_dispatch_promotes_drag_state` — `FastResizeStart`
  with hit context transitions DragState appropriately.
- `right_mouse_press_pending_then_drag_dispatches_fast_resize` —
  end-to-end mouse handler test.
- `mode_resize_console_verb_dispatches_enter_resize_mode`.
- `exit_mode_from_resize_returns_to_default_keeps_selection`.
- `resize_handle_inset_centers_match_handle_positions_minus_inset` —
  pin the 2px inset against the math.
## 7. Test plan

This section is the master test plan for the whole overhaul.
Per-batch test additions are listed inline in §3-§6 above; this
section is the integration / regression layer that catches drift
between batches.

### 7.1 Coverage table by subsystem

The table below pairs each capability with the test(s) that lock
it in. New tests are flagged `(NEW)`. Tests ride in the same commit
that introduces the code they test, per CODE_CONVENTIONS §11.

| Capability | Test file:line (or NEW) |
|---|---|
| `InteractionMode::Default` is the default | NEW: `interaction_mode.rs::tests::default_is_default` |
| Mode predicates resolve correctly | NEW: `interaction_mode.rs::tests::*` (one per predicate) |
| `Reparent` / `Connect` migration from AppMode | Existing dispatch tests under `keybinds/tests.rs` need a one-line rename ✓ |
| `EnterNodeEdit` short-circuits on single-section nodes | NEW: `dispatch/cross_dispatch/lifecycle.rs::tests::enter_node_edit_single_section_opens_editor` |
| `EnterNodeEdit` on multi-section node enters mode without opening editor | NEW |
| `EnterSectionEdit` requires NodeEdit context | NEW |
| `ExitMode` from SectionEdit drops to NodeEdit | NEW |
| `ExitMode` from NodeEdit drops to Default | NEW |
| `ExitMode` from Resize keeps selection | NEW |
| Default-mode click on multi-section node sets Single | NEW: `app/click.rs::tests::click_in_default_sets_single_even_for_multisection` |
| NodeEdit-mode click on multi-section node sets Section | NEW: `app/click.rs::tests::click_in_node_edit_sets_section` |
| NodeEdit-mode click outside active node exits mode | NEW |
| Section frame emission only in NodeEdit mode | NEW: `scene_builder/tests/section_frame.rs` |
| Inactive-node dimming in NodeEdit mode | NEW |
| Status bar shows the active mode + target | NEW |
| `Flag::Focused` set on edited section, unset on close | NEW |
| Resize handles emit only in Resize mode | NEW: `scene_builder/tests/resize_mode.rs::handles_emit_only_in_resize_mode` |
| Resize handle hit-test only fires in Resize mode | NEW |
| `EnterResizeMode` resolves selection to target | NEW (one test per selection variant) |
| `infer_resize_anchor` picks correct quadrant | NEW: `section_resize_handle.rs::tests::infer_resize_anchor_quadrants` |
| `MouseGesture::RightDrag` dispatches `FastResizeStart` | NEW |
| Fast-resize threshold-cross transitions DragState | NEW: `app/event_cursor_moved.rs::tests::right_drag_threshold_cross_promotes_to_throttled_resize` |
| Fast-resize on multi-section node targets the hit section | NEW |
| Section verb redesign: `move dx=K dy=K` | NEW |
| Section verb redesign: `move x=K y=K` (absolute) | NEW |
| Section verb redesign: `resize w=K h=K` | rewrites existing positional tests |
| Section verb redesign: `resize fill` | replaces `resize none` test |
| Section verb redesign: `text "..."` | NEW |
| Section verb redesign: `add` | NEW |
| Section verb redesign: `delete` | NEW |
| Section verb redesign: `split` | NEW |
| Section verb redesign: `show` | NEW |
| Section verb redesign: `edit` | NEW |
| `add_section` / `delete_section` / `split_section` doc setters | NEW (each with `EditNodeStyle` undo round-trip) |
| Border verb redesign: `preset cycle` | NEW |
| Border verb redesign: `side <which> <pattern>` | rewrites existing top= test |
| Border verb redesign: `corner <which> <glyph>` | rewrites existing tl= test |
| Border verb redesign: `preview` lifecycle | NEW (3 tests: set, commit, cancel) |
| Border verb redesign: `preview drops on selection change` | NEW |
| `border side` against non-custom preset errors | NEW (replaces auto-promote success) |
| `canvas border preset` writes canvas default | NEW |
| `mode` console verb (show / default / resize / etc.) | NEW |

### 7.2 Integration scenarios

These end-to-end scenarios are scripted as console-driven tests in
`src/application/console/tests/integration.rs` (new file). Each
scenario is one `#[test]` that sequences several console verbs and
asserts at the end.

1. **Edit a section in a multi-section node from the console only:**
   ```
   doc with two-section node "0"
   run "node edit" → InteractionMode::NodeEdit { "0" }
   run "section edit 1" → TextEditState::Open { node_id: "0", section_idx: 1 }
   keystrokes (simulated) → buffer = "Hello"
   run "mode default" → buffer committed via set_section_text_and_runs;
                        InteractionMode::Default; sections[1].text == "Hello"
   ```

2. **Build a multi-section node from a single-section one:**
   ```
   doc with single-section node "0", text "Original"
   run "section add at=1 text=\"second\""  → sections.len() == 2
   run "section show section=1"            → emits the readout
   run "section delete section=1"          → sections.len() == 1
   doc.undo()                              → sections.len() == 2 (delete reversed)
   doc.undo()                              → sections.len() == 1 (add reversed)
   doc.undo()                              → no-op (initial state)
   ```

3. **Border preview lifecycle:**
   ```
   doc with one node "0", style.border = None
   run "border preview preset=heavy"  → border_preview = Some(...)
   scene rebuild shows heavy border on node 0
   doc.mindmap.nodes["0"].style.border IS STILL None (no model write)
   run "border preview commit"  → set_node_border_config called; border = Some(heavy);
                                  border_preview = None
   doc.undo()                   → border = None
   ```

4. **Resize mode entered and exited:**
   ```
   doc with single-section node "0"
   doc.selection = Single("0")
   run "mode resize"  → InteractionMode::Resize { Node("0") }
   scene rebuild shows 8 handles on node 0
   simulate press at handle SE position
   simulate move past threshold
   simulate release
   set_node_aabb called; mode still Resize (drag commits, mode persists)
   simulate press in empty canvas
   simulate release
   InteractionMode::Default; selection still Single("0")
   ```

5. **Fast-resize via Ctrl+RightDrag on a multi-section node:**
   ```
   doc with two-section node "0"; sections[1].size = Some(100, 50)
   doc.selection = Single("0")
   simulate Ctrl held + right-button press at canvas position inside sections[1]'s SE quadrant
   simulate cursor move past 5px threshold
   FastResizeStart dispatched; DragState::Throttled(SectionResize) with side=SE
   simulate cursor move dx=10, dy=10
   tree-side section AABB grows to (110, 60)
   simulate release
   set_section_aabb commits; sections[1].size == Some(110, 60)
   doc.undo() → Some(100, 50)
   ```

6. **Auto-resize-handle bug regression test:**
   ```
   doc with single-section node "0"
   doc.selection = Single("0")
   InteractionMode::Default
   scene = build_scene()
   assert!(scene.node_resize_handles.is_empty())
   ```

7. **Touch parity:**
   ```
   simulate Touch::Press at node "0" with finger held for 350ms (LongPress)
   action_for_gesture("longpress", ...) returns Action::EnterResizeMode
   InteractionMode::Resize { Node("0") }
   ```
   (Pending touch recognizer implementation — this scenario is
   wired up in the touch-recognizer batch, §8.)

### 7.3 Regression-prone areas

Per CLAUDE.md guidance ("the renderer and dispatch layer cannot be
covered by the test suite alone"), these areas need manual smoke
after every batch:

- Native + WASM both: load `maps/testament.mindmap.json`, verify
  every test scenario above by hand, plus:
  - Esc key behaves correctly at every depth (Default / NodeEdit /
    SectionEdit / Resize / Reparent / Connect).
  - Status bar appears and disappears correctly.
  - Section frames render at the right positions in NodeEdit.
  - Inactive-node dimming looks right.
  - Border preview previews correctly without committing.
  - Right-mouse-drag with Ctrl held resizes from any quadrant.
  - The same right-mouse-drag without Ctrl does nothing (correct
    fallthrough).

### 7.4 Performance budget

The following are added to `benches/`:

- `bench_scene_rebuild_with_node_edit_mode_active` — scene rebuild
  on a 100-node map with NodeEdit mode active, measuring section
  frames + dimming overhead.
- `bench_scene_rebuild_with_resize_mode_active` — resize mode +
  100-node map.
- `bench_fast_resize_anchor_inference` — pure-function bench for
  `infer_resize_anchor` (sub-nanosecond expected).
- `bench_section_frame_emission` — section frames built for a
  many-section node.

Per the existing criterion bench harness in
`/home/user/mandala/benches/`. Acceptance: no measurable regression
on existing benches; new benches establish baselines.

## 8. Implementation sequence

### 8.1 Sequencing principles

- **Each batch is independently shippable.** Stop after any batch
  and the codebase is strictly better than before — though the user-
  facing surface may show partial features (e.g. Resize mode lands
  before fast-resize gesture; users see the new mode but only the
  anchor-driven path works).
- **Each batch ends with a green `./test.sh`.** Test additions ride
  in the same commit as the code they test (CODE_CONVENTIONS §11).
- **Visual smoke required.** After every batch, the implementer
  loads `maps/testament.mindmap.json` on native and WASM and runs
  through the relevant integration scenario from §7.2.
- **`./build.sh` green** before commit (cross-platform check).
- **No half-features.** A batch that introduces a new Action variant
  must include the dispatch arm, default keybind, and at least one
  reachable trigger. Per CODE_CONVENTIONS §5.

### 8.2 Batches

#### Batch 1 — Mode infrastructure (mechanical foundation)

Lands the `InteractionMode` enum and migrates `AppMode` consumers to
it. **Does not change visible behavior** — `Reparent` and `Connect`
work exactly as today; `Default` is the only other variant present.
NodeEdit / SectionEdit / Resize variants are defined but not yet
reachable.

Tasks:
- [x] Create `src/application/app/interaction_mode.rs` with the
      enum, ResizeTarget, predicate methods. Variants beyond Default
      / Reparent / Connect are defined but their predicate bodies
      are stubs (`Resize::resize_handle_*` works; NodeEdit's
      `click_resolves_to_section` works; nothing further) — wired in
      Batches 2 / 3.
- [x] Replace `app_mode: AppMode` with `interaction_mode:
      InteractionMode` across `run_native.rs`, `run_native_init.rs`,
      `input_context.rs`, `input_context_core.rs`, `event_*` handlers,
      `dispatch/native.rs`, `app/click.rs`. The `WasmInputState`
      gained an `interaction_mode: InteractionMode` field too —
      Reparent / Connect modes are cross-platform from this commit
      forward.
- [x] Delete the old `enum AppMode` at `app/mod.rs:352-367`.
- [x] Update doc comments in `keybinds/action/mod.rs`, `dispatch/*`,
      and `input_context_core.rs` for the rename.
- [x] Lift `Reparent` / `Connect` modes to be cross-platform — the
      `interaction_mode` field is on both `InitState` (native) and
      `WasmInputState` (WASM). Mode transitions still go through the
      native-only Action arms today (those depend on click hit-test
      paths only available natively for now).
- [x] Add tests for the predicate methods — 8 tests in
      `interaction_mode.rs::tests` covering `intercepts_left_click`,
      `click_resolves_to_section`, `resize_handle_node`,
      `resize_handle_section`, `is_target_picker` for every variant.
      *(Since superseded: #41 deleted `intercepts_left_click` and
      its tests — the predicate never gained a production caller.
      The other four predicates and their tests stand. Recorded
      here rather than edited out; the checklist is history.)*

Verification: `./test.sh` green (2274 → 2282 tests, +8 from new
predicate tests). `./test.sh --lint` advisory clippy warnings are
all pre-existing. WASM target compiles cleanly (two pre-existing
unused-import warnings on the platform shim — separate concern).

#### Batch 2 — Resize mode (the urgent UX fix) — SHIPPED

Tasks (status):
- [x] Mode-driven gate at `document/mod.rs:520-523` reads from
      `ResizeHandleOverrides` populated from `InteractionMode`. Auto-
      anchor-on-selection bug FIXED — `Single`/`Section` selections
      in `Default` mode emit zero handles.
- [x] Press-time handle hit-test gates moved from selection
      (`event_mouse_click.rs:309-345`) to mode
      (`interaction_mode.resize_handle_*()`).
- [x] `Action::EnterResizeMode` + `apply_enter_resize_mode` helper
      in `cross_dispatch::lifecycle`.
- [x] Default keybind `enter_resize_mode: vec!["r".into()]`.
- [x] `CancelMode` (Esc) extended to exit Resize → Default. The
      `CancelMode → ExitMode` rename is **deferred** — the
      functional behavior ships now; the rename is a separate
      drive-by since user keybinds.json files would break and the
      semantic value is small.
- [x] `HANDLE_HIT_TOLERANCE_PX` 12 → 8.
- [x] Corner inset 2px **deferred** (per plan §6.9 itself flags this
      as "minor follow-up; not strictly required to fix the bug").
      Existing `resize_handle_positions` tests pin specific
      positions; insetting requires a coordinated test update.
- [x] `mode` console verb with `show | default | resize` subverbs
      via new `ConsoleSideEffect::SetInteractionMode`.
- [x] Tests: 9 new (regression `default_mode_with_single_selection_emits_no_resize_handles`,
      counterpart `resize_mode_node_target_emits_eight_handles`,
      6 `mode` console verb tests, 1 InteractionMode predicate test
      for resize_handle_overrides).

2274 (baseline) → 2282 (Batch 1) → 2291 (Batch 2). Native + WASM
both green.



Wires NodeEdit-and-Resize-mode-driven handle gating, lands the
`EnterResizeMode` Action, the `mode resize` console verb, and
`ExitMode`. **No** fast-resize gesture yet (Batch 4); **no**
NodeEdit visuals yet (Batch 3) — handles are reachable in Resize
mode only. The auto-anchor-on-selection bug is fixed here.

Tasks:
- [ ] Wire `InteractionMode::Resize` predicates in
      `interaction_mode.rs`. Add `Resize { target: ResizeTarget }`
      handling.
- [ ] Change `document/mod.rs:520-523` to gate on
      `interaction_mode.resize_handle_*()` (this is the one-line
      bugfix).
- [ ] Change `event_mouse_click.rs:298-345` resize-handle hit
      gating to mode-driven (per §6.8).
- [ ] Add `Action::EnterResizeMode` + dispatch arm in
      `cross_dispatch/lifecycle.rs::apply_enter_resize_mode`.
- [ ] Add `Action::ExitMode` (rename from `CancelMode`) + dispatch
      arm; route from-Resize → Default; from-Reparent / Connect →
      Default (today's `CancelMode` body).
- [ ] Default keybind: `enter_resize_mode: vec!["r".into()]`.
- [ ] Reduce `HANDLE_HIT_TOLERANCE_PX` 12 → 8.
- [ ] Inset corner handles by `RESIZE_HANDLE_INSET_PX = 2.0`.
- [ ] Add `mode` console verb (just `show | default | resize`
      subverbs in this batch — the rest are added in their respective
      batches).
- [ ] Tests per §7.1 (resize handle gating, EnterResizeMode resolution).

Verification: tests green; manual smoke — open testament, click a
node, verify NO handles appear; press `r`, verify 8 handles appear;
drag a handle, verify resize works; press Esc, verify handles
vanish; verify selection preserved.

#### Batch 3 — NodeEdit mode visuals + section selection routing — SHIPPED

Wires NodeEdit mode end-to-end: section frames, dimming, status bar,
click routing. The user can now enter NodeEdit mode (`n` keybind or
`mode node-edit`) and click sections to select them. Section text
editor is reachable via `Action::EnterSectionEdit` (Enter from
NodeEdit context).

Tasks (status post-Tier-1/2 review fixes):
- [x] `InteractionMode::NodeEdit { node_id }` predicates wired.
- [x] `app/click.rs` click routing consults
      `interaction_mode.click_resolves_to_section(...)`.
- [x] `Action::EnterNodeEdit` / `EnterSectionEdit` /
      `EnterNodeEditClean` shipped (renamed from `EditSelection*`).
      The `EditSelection` umbrella stays as a rename-with-shim
      (documented divergence from §3.8 — it dispatches to
      `EnterNodeEdit` / `EnterEdgeLabelEdit` / `EnterPortalTextEdit`
      based on selection variant; allows the existing keybind
      to keep working without a forced rebind).
- [x] Single-section short-circuit in `apply_enter_node_edit`
      (opens editor + sets mode in one pass; `exit_to_default_on_close`
      so the user lands at Default after editing).
- [x] `EditSelection*` arms dispatch through `EnterNodeEdit*`.
- [x] `InputContext::NodeEdit` variant in `keybinds/context.rs`.
- [x] Modal-stealer cascade branch for NodeEdit in
      `event_keyboard.rs`.
- [x] `enter_node_edit: vec![]` keybind default (left empty;
      the `edit_selection: vec!["Enter".into()]` keybind keeps
      working via the umbrella dispatch). `EditSelection`'s
      umbrella dispatch covers the documented intent.
      `enter_section_edit: vec!["Enter".into()]` default in
      NodeEdit context. (Documented divergence from §3.8: the
      plan called for `edit_selection` to be deleted; we kept
      it as the umbrella entry point, which is functionally
      equivalent and avoids forcing an existing-config rebind.)
- [x] `scene_builder/section_frame.rs` section-frame pass.
- [x] Inactive-node dimming in `scene_builder/node_pass.rs`.
- [x] Status-bar overlay in `scene_host.rs`.
- [x] `section edit` console subverb shipped (Batch 5 deferred
      3/N, commit `b84c00f`). `node edit` console subverb
      **deferred** — `mode node-edit` covers the same intent
      via the existing `mode` verb; a `node edit` alias is
      pure sugar.
- [x] `Flag::Focused` set on active section in
      `apply_text_edit_to_tree`.
- [x] Outside-click exits NodeEdit handler in
      `event_mouse_click.rs`.
- [x] Tests per §7.1 (NodeEdit-related rows pinned).

Open follow-ups (deferred to next PR):
- [ ] §4.7 hover affordance — `hovered_section: Option<(String, usize)>`
      on `InitState` plus a 1.2× brightness section-frame pass.
      Not load-bearing for the NodeEdit UX (the editor cycle
      works without hover); ship in a follow-up Batch-3.5 or
      fold into Batch 7's touch parity work.
- [ ] `node edit` console subverb (sugar over `mode node-edit`).

Verification (post-Tier-1/2 review fixes): 2544 tests green;
wasm32 cross-compile clean.

#### Batch 4 — Fast-resize gesture — SHIPPED

Adds RightClick / RightDrag MouseGesture variants and the
Ctrl+RightDrag fast-resize gesture. No new visual chrome (the
gesture works against any node, including not-currently-selected
ones). Touch is deferred to Batch 7.

Tasks (status post-9-agent review fixes):
- [x] `MouseGesture::RightClick` / `RightDrag` in `bind.rs`.
- [x] `MouseButton::Right` press / release arms in
      `event_mouse_click.rs` (separate `DragState::PendingRight`
      variant carrying press-time hit + canvas pos for press-time
      quadrant inference).
- [x] Threshold-cross arm in `event_cursor_moved.rs` dispatches
      `Action::FastResizeStart` with the press-time hit and
      canvas pos in `DispatchHit`.
- [x] `Action::FastResizeStart` + dispatch arm; computes
      anchor via `infer_resize_anchor`; transitions DragState
      into `Throttled(NodeResize | SectionResize)`. Marked
      `destructive` per §6.10.
- [x] `infer_resize_anchor` in `scene_builder/section_resize_handle.rs`.
- [x] Default keybind `fast_resize_start: ["Ctrl+RightDrag"]`
      (kept on `Vec<String>` since the Action takes no payload).
- [x] Cursor icon plumbing via `cursor_icon_last: CursorIcon`
      on `InitState` + `cursor_icon_for_resize_side` mapping.
- [x] WASM `contextmenu` event suppression (with Shift+RightClick
      bypass for browser-context-menu access).
- [x] Tests: 8 new (anchor math, gesture round-trips, cursor
      mapping pin, keybind resolution, default destructive set).

Verification: 2523 tests green post-9-agent review fixes;
wasm32 cross-compile clean.

#### Batch 5 — Section console verb redesign + new doc setters — SHIPPED

Lands the new `section` verb grammar and the `add_section` /
`delete_section` / `split_section` doc setters.

Tasks (status post-Full-Nelson review):
- [x] Section verb grammar lives in `console/commands/section/`
      (`mod.rs` + `frame.rs`); per-subverb-file split deferred —
      the single `mod.rs` ~700 LoC houses every subverb's
      `execute_*` and the shared parsers, mirroring the shape
      Batch 6's `border/` settled on after its own evolution.
- [x] Doc setters `add_section` / `delete_section` /
      `split_section` shipped in `nodes/section_structure.rs`
      with full undo discipline (`EditNodeStyle` extended with
      `before_position` / `before_size`).
- [x] kv-form migration: `move dx=/dy=`, `move x=/y=` (NEW
      absolute), `resize w=/h=`, `resize fill` (renamed from
      `none`).
- [x] New subverbs: `show`, `text`, `add`, `delete`, `split`.
      `text` honors `runs=preserve|clear` (preserve uses the
      new `set_section_text_preserving_runs` helper; clear
      collapses via `set_section_text`).
- [x] `node_or_section_selected` predicate added in
      `predicates.rs:33`; `Multi(_)` excluded to avoid
      predicate-vs-runtime mismatch flagged by the Full-Nelson
      review.
- [x] §4.5 rule 3: `Single(id)` on a single-section node
      auto-resolves to `(id, 0)` (closes the §5.7 hostile
      error).
- [x] `format/sections.md` rewritten with the 9-subverb table
      (post-`section edit`-ship; the 9th subverb landed in the
      deferred-items follow-up).
- [x] §4.5 rule 4: MultiSection fan-out for `move dx=X dy=Y`
      shipped in commit `ff22f5c`. Atomic parse-then-dispatch
      via `MindMapDocument::validate_section_offset_change` —
      the verb pre-validates every selected pair's would-be
      AABB; a single rejection aborts the whole fan-out so
      partial mutation never lands. Other subverbs
      (`text` / `resize` / `delete` / `split`) keep
      single-target rejection on MultiSection.
- [x] §4.6 Action variants — `SetSectionOffsetAbs`,
      `SetSectionText`, `AddSection`, `DeleteSection`,
      `SplitSection { at_grapheme }` — shipped in commit
      `256c096`. Macro-only targets today (no `KeybindConfig`
      fields; the string-arg payloads make keybinding awkward).
      Doc-comments on the 5 variants explicitly say "macro-only
      target" so future readers don't assume keybind reach.
      The 4 destructive ones (`SetSectionText`, `AddSection`,
      `DeleteSection`, `SplitSection`) are `#[action(destructive)]`
      and pinned in `keybinds/tests.rs::test_is_destructive_destructive_set_is_pinned`.
- [x] `section edit [section=<idx>]` subverb shipped in commit
      `b84c00f`. Routes through the new
      `ConsoleSideEffect::OpenSectionEdit { node_id, section_idx }`
      bus variant; the post-rebuild handler delegates to the
      canonical `apply_enter_section_edit` (the same path
      `Action::EnterSectionEdit` uses on the keybind side) for
      `OwnerMismatch` validation and consistent posture.

Verification (post-deferred-items + Tier-1/2 review fixes):
2544 tests green; wasm32 cross-compile clean.

#### Batch 6 — Border verb redesign + canvas-default editing + preview — SHIPPED

Lands the new `border` verb grammar, the `border preview` lifecycle,
and the new `canvas border` verb.

Tasks (status post-Batch-6 ship):
- [x] `console/commands/border/` already partially modular
      (`mod.rs` / `complete.rs` / `execute.rs` / `preview.rs` /
      `show.rs` / `tests.rs`). Further per-subverb file split
      is pure refactor (no behavior change); deferred — the
      shape today is workable.
- [x] Subverb-routed positional parsers added in B6.2-6
      (preset / color / padding / palette / font / side /
      corner / toggle). Kv form preserved as the keybind-
      friendly alias per Plan §5.2.
- [x] `BorderPreview` field on `MindMapDocument` shipped in
      Batch 5/Tier-1 ship (preview lifecycle was working at
      Batch-5 time per §5.6 status). Scene rebuild already
      threads `border_preview: Option<BorderPreview<'a>>`
      through `build_scene_with_cache`.
- [x] **Partial** Action variants per §5.8: `CycleBorderPreset`
      and `ToggleBorderVisible` (the no-payload ones) shipped in
      B6.9. The 7 String-payload variants
      (`SetBorderPreset(String)` / `SetBorderColor(String)` / ...)
      deferred — they overlap with the existing
      `SetBorderField { field, value }` parametric variant whose
      deletion (Plan §5.8 last paragraph) is a breaking-change
      migration for every parametric keybind binding shape.
      Tracking as a follow-up; today's `SetBorderField` covers
      the same surface for keybinds.
- [x] `applicable: always` → `node_or_section_selected` (B6.1).
- [x] Auto-promote-to-custom guarded at the verb layer (B6.7).
      The data-layer auto-promote stays as the model invariant
      defense ("glyphs only render with preset=custom") so
      macro consumers and the kv form continue to work; the
      verb-layer pre-check makes the user-facing positional
      path error explicitly per Plan §5.4 #3.
- [x] `canvas` top-level verb already shipped (`canvas.rs`,
      pre-Batch-6) with `border` and `section-frame [focused]`
      subjects. B6.10 extends the verb with the Plan §5.7
      positional subverb grammar matching the per-node `border`
      verb.
- [x] `set_canvas_default_border_config` doc setter already
      exists. **`EditCanvasStyle` undo variant** subsumed by
      the existing `UndoAction::CanvasSnapshot` which captures
      the entire `Canvas` (palettes, defaults, theme vars) in
      one entry — same round-trip contract Plan §5.7 calls for,
      just stored as a snapshot rather than a per-field diff.
- [x] Tests migrated; new tests added per §7.1 (28 new pins
      across B6.1-10).
- [x] `format/border-patterns.md` Console verb section
      rewritten (B6.11) to surface positional subverbs first,
      kv form as the keybind alias, and the `border side` /
      `border corner` non-custom-preset error.

Verification (post-Batch-6 ship + opus review remediation):
2646 tests pass; wasm32 cross-compile clean.

##### Open follow-ups flagged by the opus review

Honest deferrals that the original Batch-6 ticking missed.
Track here so Batch 8 (or earlier follow-ups) can pick them up:

- **§5.5 `BorderEditOutcome` removal**: the spec calls for
  removing the bespoke `BorderEditOutcome` and routing through
  `helpers::ApplyTally::finalize`. Still present at
  `border.rs:118-133`, returned by 4+ setters. Substantial
  refactor across every border setter; not Batch-6-shipping.
- **§5.5 typed `Outcome::Lines` from the `preset` subverb**:
  spec calls for the auto-promote message to be emitted by
  the `preset` subverb's success path. Today the message
  still rides via `apply_edits`'s shared formatter (which
  fires regardless of which subverb invoked it).
- **§5.4 #3 verb-strict vs macro-permissive**: the verb-layer
  `border side|corner` now errors on non-custom presets. The
  data-layer auto-promote (`apply_glyph_border_edits_to_slot`)
  stays as the model invariant defense, so
  `Action::SetBorderField { field: "top", value: "..." }` from
  a macro still silently auto-promotes. Deliberate (kv-form
  back-compat); pin a regression test in `macros/tests.rs`
  that names the verb-strict-vs-macro-permissive contract so
  a future contributor doesn't tighten one without the other.
- **§5.7 doc-setter naming**: spec calls for
  `set_canvas_default_border`; reality is
  `set_canvas_default_border_config`. Cosmetic; rename in a
  follow-up commit.
- **§5.8 7 of 9 typed Action variants**: `SetBorderPreset(String)`
  / `SetBorderColor(String)` / `SetBorderPadding(String)` /
  `SetBorderPalette { palette, field }` / `SetBorderFont {
  family, size_pt }` / `SetBorderSide { side, pattern }` /
  `SetBorderCorner { corner, glyph }`. `SetBorderField`
  preserves the keybind surface for now; Batch 8 should land
  the typed variants and `#[deprecated]` `SetBorderField`.
- **§5.9 completion templates**: the rendered-in-border-font
  pattern templates for `border side WHICH <TAB>` (6 templates)
  and the glyph candidates for `border corner WHICH <TAB>`
  (13 candidates) need a typed catalogue + font-renderer
  integration; the `reset` completion shipped in T4 covers
  the high-value discoverability gap.
- **§5.10 inline action hints in `border show`**: shipped in T5
  with the `(toggle: ...)` / `(cycle: ...)` / `(override: ...)`
  annotations.
- **§5.11 test migration**: kept additive (kv-form tests still
  green alongside positional-form tests). Plan called for
  rewrite; the additive shape catches both regressions and
  costs little.
- **canvas verb parity**: `canvas border show` doesn't accept
  `side=` filter / `verbose` flag (only the per-node `border
  show` does); `canvas border preset cycle` not supported.
  Cosmetic asymmetries; document or extend.

#### Batch 7 — Touch parity — SHIPPED

Lands the touch gesture recognizer and the `LongPress` /
`TwoFingerDrag` MouseGesture variants. Touch input becomes a peer
of mouse — long-press is the touch equivalent of `r`
(EnterResizeMode), two-finger-drag is the touch equivalent of
Ctrl+RightDrag (FastResizeStart). Pre-Batch-7 the WASM event
loop dropped `WindowEvent::Touch` silently, leaving mobile-browser
users with no way into either resize gesture.

Tasks (status post-Batch-7 ship):
- [x] `MouseGesture::LongPress` / `TwoFingerDrag` variants
      added to `keybinds/bind.rs:84-103` with strum
      serializations (`"longpress"`, `"twofingerdrag"`) and
      pascal-form mappings.
- [x] `TouchGestureRecognizer` state machine landed in
      `src/application/app/touch_gesture.rs`. Pure state
      machine: `Idle` ↔ `OneFinger` ↔ `TwoFingers`; long-press
      timer fires from `tick(now)`; two-finger-drag emits per
      `POINTER_DRAG_THRESHOLD_PX` centroid step. `LONG_PRESS_MS
      = 350` (matches iOS / Android conventions). The move
      threshold landed as a touch-local `MOVE_THRESHOLD_PX = 4.0`
      claiming to match the mouse drag threshold, which was 5.0;
      #46 replaced both with the one `POINTER_DRAG_THRESHOLD_PX
      = 5.0` in `app/mod.rs`.
      Test-only `with_thresholds` constructor for state-machine
      tests so they don't sleep 350ms per case.
- [x] Native `WindowEvent::Touch` arm in `run_native.rs:454-462`
      → `dispatch_touch_event` helper at line 295 builds the
      `Phase` translation, drives the recognizer's `ingest` +
      `tick`, looks up the bound action via
      `keybinds.action_for_gesture` (modifier-fixed-false), and
      dispatches through `super::dispatch::dispatch_action`.
      `touch_recognizer: TouchGestureRecognizer` field on
      `InitState`.
- [x] WASM `WindowEvent::Touch` arm in `run_wasm/mod.rs:454-456`
      → new `event_touch.rs` sibling with `handle_touch_event`
      mirroring native. Uses
      `dispatch::action_core::dispatch_compatible` (the cross-
      platform dispatcher; FastResizeStart is `NativeOnly` on
      WASM and falls through to a warn-log via the existing
      Native-arm gate).
- [x] Default keybinds extended:
      `enter_resize_mode: ["r", "LongPress"]`,
      `fast_resize_start: ["Ctrl+RightDrag", "TwoFingerDrag"]`
      in `keybinds/config.rs:297-298`. Four new pin tests in
      `keybinds/tests.rs` lock the resolution + JSON-shape so
      a regression that drops either touch entry trips at
      config-resolution time.
- [x] State-machine tests: 14 new pins covering long-press
      fire / sub-threshold survival / movement-cancel /
      finger-lift-cancel / second-finger-cancel /
      two-finger centroid emission / sub-threshold no-fire /
      per-step emission / lifting-one-of-two / Idle-after-both /
      third-finger-ignored / `reset()` / RecognizedGesture →
      MouseGesture mapping / untracked-finger-id ignored.

Long-press wake-up gap (acknowledged): the recognizer's `tick`
fires only from `WindowEvent::Touch` events, not on a wall-clock
timer. A finger held with literally zero `Moved` events between
Started and Ended would miss the long-press emission. In
practice touch hardware emits sub-pixel jitter `Moved` events
constantly while a finger is down, so the gap is theoretical.
A future improvement would set
`ControlFlow::WaitUntil(started_at + LONG_PRESS_MS)` from the
recognizer's `OneFinger` state — deferred to keep Batch 7's
diff tractable.

Verification: 1608 tests pass on `cargo test --bin mandala`
(was 1590 → +18: 14 state-machine pins + 4 keybind-default
pins). Wasm32 cross-compile clean. Manual verification on a
touch device is **out of scope** for this session — no device
available. The state-machine tests pin the recognition logic;
the wiring is a thin call-through layer verified by reading
the diff.

#### Batch 8 — Documentation, polish, drive-by fixes — SHIPPED

Lands the doc updates, deletes deprecated paths fully, runs a
final pass to surface any seams the previous batches noticed but
deferred. Per CODE_CONVENTIONS §5 (drive-by fixes).

Tasks (status post-Batch-8 ship):
- [x] `CONCEPTS.md` extended with `### `InteractionMode`` (the
      three-mode lifecycle Default / Resize / NodeEdit) and
      `### `SectionFrameElement` and section-frame chrome` (the
      cyan-rectangle parallel-canvas dispatch). Both cross-ref
      §5 "The application runtime" and the plan's §2-§4 design.
- [x] `format/sections.md` — already updated in Batch 5 + Tier 3
      review fixes. Verified accurate (9-subverb table, fan-out
      atomicity note, schema-drift acknowledgement).
- [x] `format/border-patterns.md` — rewritten in Batch 6 + T6 of
      the opus review (positional grammar surfaces first, kv as
      keybind alias, non-custom-preset error documented, cycle
      semantics noted, internal field-name leak fixed).
- [x] `CLAUDE.md` "Common tasks" verified accurate — every
      flag listed (`--coverage` / `--lint` / `--bench`) matches
      `./test.sh --help`.
- [x] README verified — high-level orientation unchanged; no
      user-facing capability claims drifted.
- [x] `cargo doc --workspace --no-deps`: 12 warnings pre-fix → 0
      warnings post-fix. Each broken-link / private-item-link /
      unclosed-html-tag site fixed at the source (B8.1+5 commit).
- [x] REFACTOR_PLAN.md Batch 5.2 (DragState `Pending` enum
      conversion) audited — the deferral note's release-UX
      rationale still stands; the Section/Border PR didn't
      touch `Pending` (Batch 4 added a parallel `PendingRight`
      variant; Batch 5/6 ride the `Throttled(SectionResize)`
      path). Time isn't right; deferral re-confirmed.
- [x] `// FIXME` / `// TODO` / `// HACK` sweep — zero hits across
      `src/` and `lib/baumhard/src/`.
- [x] `cargo bench` for the Plan §7.4 benches:
      `fast_resize_anchor_inference` ~360 ps/call (no regression
      vs Batch 4 baseline); `scene_rebuild_node_edit_mode_active`
      ~30 µs/rebuild on the 50-node × 5-section synthetic;
      `section_frame_emission_50x5_with_node_edit_active`
      ~3.3 µs.

Drive-by fixes from the Batch-6 opus review folded in:
- [x] §5.7 doc-setter rename `set_canvas_default_border_config`
      → `set_canvas_default_border` (mechanical, 9 sites in 4
      files; per CODE_CONVENTIONS §10 no shim).
- [x] Verb-strict vs macro-permissive contract pin
      (`apply_border_field_to_selection_auto_promotes_preset_to_custom`
      in border/tests.rs) — locks in the deliberate divergence
      between the verb-layer error and the data-layer auto-
      promote so future tightening doesn't drift one without
      the other.
- [x] `canvas border preset cycle` parity — single-line
      addition in `positional_subverb_to_edits`; same wrap
      order as the per-node verb. Two new tests
      (`canvas_border_preset_cycle_advances_canvas_default`,
      `canvas_section_frame_preset_cycle_wraps`).

Open follow-ups deferred to a future PR (acknowledged in plan's
§5.B6 "Open follow-ups" block):
- [ ] §5.5 `BorderEditOutcome` removal + `helpers::ApplyTally::finalize`
      reuse — substantial refactor across every border setter.
- [ ] §5.5 typed `Outcome::Lines` from `preset` subverb — depends
      on `BorderEditOutcome` removal.
- [ ] §5.8 7 of 9 typed `Action` variants (`SetBorderPreset(String)` /
      `SetBorderColor(String)` / etc.) + `#[deprecated]` on
      `SetBorderField`. `SetBorderField` preserves the keybind
      surface today.
- [ ] §5.9 rendered pattern templates for `border side WHICH <TAB>`
      (6 templates in border font) and glyph candidates for
      `border corner WHICH <TAB>` (13 candidates) — needs a typed
      catalogue + font-renderer integration. The `reset` second-
      positional row shipped in T4.
- [ ] `canvas border show side=` / `verbose` parity — the canvas
      show path uses a custom formatter (no node `size`, no dual
      color cascade), so adding the per-node flags would force-
      fit a different output shape. Documented as honest
      asymmetry.
- [ ] §3.8 `enter_node_edit` keybind default — kept
      `edit_selection` as the umbrella entry point instead of
      deleting it; functionally equivalent, avoids a forced
      config rebind on existing users.
- [ ] §4.7 hover affordance — per-frame brightness modulation;
      better folded into Batch 7's touch-parity work.
- [ ] `Arc<str>` migration for `SectionFrameElement.node_id`
      (Performance #2 from the §4.6 review).
- [ ] Resource caps on text length / sections per node / undo
      depth (Security I-1/2/3 from the §4.6 review).
- [ ] §3.9 `node edit` console subverb — sugar over `mode
      node-edit`; landed.
- [x] `SectionRange` grapheme-vs-section type confusion —
      resolved by #47 part C: the variant now carries
      `section_span: SectionSpan` and `grapheme_range:
      GraphemeRange` as distinct newtypes, and
      `lift_anchor_to_section_range` emits the variant with both
      fields end-to-end.

Architectural follow-ups from the **whole-PR** opus review T4 —
acknowledged honestly, deferred because each is invasive (touches
≥6 callsites) and none fixes a correctness bug:

- [ ] **Layering inversion** — `app/dispatch/cross_dispatch/style.rs`
      reaches *up* into `console::commands::border` for its mutation
      cores (`apply_border_field_to_selection`, `nodes_in_selection`,
      `stage_kv`, `BorderConfigEdits`, `BorderPreviewTarget`). The
      "correct" shape moves these to `document/nodes/border.rs` (or
      a new `mutation_cores` module) and makes the verb layer a thin
      shell over them. The verb layer doubling as the mutation-core
      registry is a known seam — `style.rs:22/32/55/65/104/122` shows
      6 reach-ups today. Pure refactor; no behavior change.
- [ ] **border/canvas/section apply-path dedup** — the three apply
      paths (`border/execute.rs::apply_edits`,
      `section/frame.rs::apply_section_frame_edits`, the inline
      canvas.rs apply paths) share kv-staging + auto-promote +
      bare-custom-hint logic. `stage_kv` is already shared
      (post-Batch-6 review-fix); the dispatcher shapes are 80%
      identical but each parses targets differently
      (per-node ids / per-section pairs / canvas slot). A unified
      "stage edits + dispatch to target" entrypoint would collapse
      three parallel implementations. Deferred because the
      target-resolution divergence is the load-bearing 20%; the
      refactor needs a `BorderEditTarget` enum that none of the
      three call sites authored.
- [ ] **`Result<Option<T>, E>` simplification** —
      `canvas.rs::positional_subverb_to_edits` returns
      `Result<Option<BorderConfigEdits>, ExecResult>` to express
      three outcomes (apply edits / verb self-handled / error).
      Cleaner with a 3-arm enum like `enum Ctl<T> { Apply(T),
      Done(ExecResult), Error(ExecResult) }`. Smell flagged once
      but not on a hot path; cosmetic.

Final whole-PR review (10-agent Opus). Findings closed in
this commit are the BLOCKER tier (3) plus the high-value
CRITICALs / HIGHs (10 fixes; 3 new test pins; cargo doc clean).
The remainder are deferred here:

- [x] **WASM touch lift** (touch deep-dive CRITICAL + cross-
      platform B1 / C1): the shipped Batch 7 default keybinds
      (`LongPress → EnterResizeMode`, `TwoFingerDrag →
      FastResizeStart`) both bound to NativeOnly Actions, so the
      browser dispatched them, got `Unhandled`, and dropped them.
      **Closed by #35, and not by the lift this item proposed.**
      The vocabulary grew a tap, a one-finger pan and a pinch step,
      and those three resolve to no `Action` at all — they take the
      selection-bookkeeping and continuous-camera carve-outs
      CODE_CONVENTIONS §3 already grants the mouse, through one
      cross-platform `dispatch::apply_touch_effect`. So the browser
      is usable by finger without any of the DragState /
      modal-stealer port. `TwoFingerDrag` is gone from the
      `MouseGesture` table: two fingers now drive the camera, and a
      binding firing on the same event would be a conflict.
      `LongPress → EnterResizeMode` stays NativeOnly *deliberately*
      — it is the touch peer of the keyboard's `r`, and rebinding it
      to something Compatible on one target only would make one
      gesture mean two things — and keeps its warn-log. Lifting it
      is the `InteractionMode::Resize` chrome port, still open below.
- [ ] **WASM resize-handle hit-test** (cross-platform C2): even
      if `EnterResizeMode` were Compatible, WASM's left-click
      handler (`run_wasm/event_mouse_click.rs::handle_mouse_pressed`)
      lacks the `hit_test_section_resize_handle` /
      `hit_test_node_resize_handle` calls native uses. The
      visible chrome wouldn't function on click.
- [ ] **WASM `MouseGesture` coverage gaps** (cross-platform H2 / H3):
      `WheelUp/Down` ignores the keybind table, hard-coding
      `factor = 1.1`; `RightClick` only logs and never dispatches
      `action_for_gesture`. Both diverge from native semantics.
- [ ] **Long-press wake-up gap is broken not theoretical** (touch
      deep-dive CRITICAL): `tick()` is only invoked on
      `WindowEvent::Touch` events. iOS Safari coalesces events so
      a finger held with no Moved between Started and Ended will
      not fire LongPress. Wire `ControlFlow::WaitUntil(started_at +
      LONG_PRESS_MS)` on native or `setTimeout` on WASM. Still open
      for the long-press. It does **not** leak into #35's tap: the
      tap's refusal reads the elapsed time rather than whether the
      long-press emission happened, so a hold whose `tick` never ran
      is refused on duration —
      `test_tap_is_refused_for_a_long_hold_whose_timer_never_ran`.
- [ ] **Touch coordinate-space drift** (touch deep-dive HIGH):
      `winit::Touch.location` is `PhysicalPosition<f64>` but
      `POINTER_DRAG_THRESHOLD_PX` was doc-claimed as logical pixels.
      On a 2x retina device the threshold is effectively ~2
      logical px; on iPhone ~1.3 logical px. Either divide by
      `window.scale_factor()` before ingest or rename the
      constant + fix docs.
      **Half closed** (#46 review follow-up): the *doc* half is
      done — the constant, `keybinds/bind.rs` and the touch module
      now all say physical, and the constant's doc names the two
      call sites (`handle_cursor_moved`'s `cursor_pos` store and
      `dispatch_touch_event`'s `touch.location`) that feed raw
      device coordinates in, plus the fact that no `scale_factor`
      call exists anywhere under `src/application`. The item stays
      open for the half that matters to a user: the budget is still
      display-dependent, so the same gesture costs half the finger
      travel on a 2x screen. Closing it is the `scale_factor`
      division at both readers, which changes behavior on every
      HiDPI machine and wants its own change. The mouse side is
      equally affected, so the fix is per-pointer-path, not
      touch-only.
- [x] **Touch clock-skew vulnerability** (touch deep-dive HIGH):
      both readers of the elapsed hold — `tick`'s long-press test
      and the tap's duration clause — now use
      `saturating_duration_since`. (The panic half of the report was
      already wrong: `web_time`'s `duration_since` is
      `checked_duration_since(..).unwrap_or_default()`, so it
      saturated too. What the explicit spelling buys is that the
      guarantee is written down at the call site rather than
      inherited from a dependency's choice.) A regressed timestamp
      now reads as a zero-length hold, which is the right answer for
      both: too short to be a long-press, short enough to be a tap.
- [x] **Touch `Cancelled` handling** (touch deep-dive HIGH):
      `Phase` now carries its own `Cancelled`, and `touch_phase` no
      longer folds winit's onto `Ended`. The cancelled finger clears
      its slot exactly as a lift does — the survivor of a pair goes
      on being tracked — and emits nothing. Folding was harmless
      while no gesture fired on a lift; #35's tap made it a bug in
      which the operating system commits a selection every time it
      interrupts a finger, which is what
      `test_a_cancelled_touch_neither_taps_nor_leaves_state_behind`
      pins.
- [x] **Touch `sqrt` on input hot path** (touch deep-dive MEDIUM
      + CONVENTIONS violation): `update_pos` did
      `(dx*dx+dy*dy).sqrt() > MOVE_THRESHOLD_PX`. Both touch emit
      points now compare squared against
      `POINTER_DRAG_THRESHOLD_SQ_PX`, the same form the mouse arms
      use (#46).
- [ ] **Touch demote latch overcorrects** (touch deep-dive MEDIUM):
      "two fingers down, lift one quickly, hold survivor 350ms" is
      plausibly long-press intent but the demotion latches
      `discrete_emitted: true` so the survivor never fires. Open,
      and deliberately so for now: #35 widened the latch's job — it
      also refuses the survivor's *tap*, which is what stops a
      two-finger gesture from ending in a selection change — and
      narrowing it wants a rule that distinguishes "lifted one
      finger to keep holding" from "finished pinching", which the
      machine does not have. What #35 did change is that the latch
      is now the whole mechanism: the demotion used to also
      back-date the survivor's `started_at` under a comment claiming
      *that* was what blocked the long-press, which was one
      mechanism plus a decoy.
- [x] **Native + WASM touch dispatch near-duplication** (architecture
      MEDIUM): `InitState::dispatch_touch_event` and
      `WasmApp::handle_touch_event` carried near-identical bodies.
      #29 lifted the recognize-and-resolve half into
      `dispatch::drive_touch_event`; #35 lifted the *act* half into
      `dispatch::apply_touch_effect`, which both call. What is left
      per-target is the one thing that genuinely differs: native
      dispatches a keybind-routed Action through `dispatch_action`
      with its native context, the browser through
      `dispatch_compatible` plus the `NativeOnly` warn-log.
- [ ] **DOS resource caps** (security CRITICALs):
      `MindMapDocument.undo_stack`, `MindNode.inline_macros`,
      `MindMap.macros`, `Palette.groups`, and section/node text
      length are all unbounded. The `add_section` 1024-cap shipped
      in T5 is the model — mirror it for `MAX_UNDO_DEPTH`,
      `MAX_INLINE_MACROS_PER_NODE`, `MAX_TEXT_LEN`,
      `MAX_NODES_PER_MAP`, `MAX_PALETTE_GROUPS` at runtime AND
      verify.
- [ ] **`split_section` ignores 1024 cap** (correctness MEDIUM):
      `add_section` checks `MAX_SECTIONS_PER_NODE` but `split_section`
      doesn't, so repeated splits past the cap bypass the
      protection. `crates/maptool/src/verify/sections.rs:101` also
      hardcodes 1024 instead of importing the const — drift risk.
- [ ] **LMB-release origin gate asymmetry** (correctness HIGH):
      `event_mouse_click.rs:697-702` left-button release finalises
      a Throttled NodeResize/SectionResize regardless of
      `started_with_right`. The right-button release path correctly
      gates on the flag (T2 fix); LMB path doesn't. User holding
      RMB for fast-resize then fumbling LMB ends the gesture
      prematurely.
- [x] **`SectionRange::range` dual semantics** (correctness HIGH):
      resolved by #47 part C — the variant was split into two
      distinctly-typed fields (`section_span: SectionSpan`,
      `grapheme_range: GraphemeRange`); every reader consumes the
      field it means and the editor-close lift produces the
      variant with both.
- [ ] **`split_section` undo doesn't pin `text_runs`** (test coverage
      CRITICAL): `split_section_pushes_undo_and_dirty` checks
      `sections.len()` and `text` after undo, not `text_runs`.
      A regression in run-restoration would slip past.
- [ ] **MultiSection `section move` phase-1 atomicity untested**
      (test coverage CRITICAL): the "first rejection aborts the
      whole fan-out" guarantee at `section/mod.rs:765-790` has no
      test pinning the partial-mutation-impossible invariant.
- [ ] **Section count cap (1024) verify-side test missing** (test
      coverage CRITICAL): `crates/maptool/src/verify/sections.rs::check_section_count_cap`
      added in T5 has no test asserting a 1025-section node trips
      the violation.
- [ ] **`section frame` VERBS asymmetry** (API/UX HIGH):
      `section frame::VERBS = ["show", "reset", "preview"]` ships
      no positional per-field subverbs while sibling
      `border` / `canvas border` / `canvas section-frame` ship
      `preset / color / padding / palette / font / side / corner`.
      The unknown-subverb error already acknowledges this as a
      follow-up.
- [ ] **`section: unknown subverb '{}'` second site** (API/UX HIGH):
      the late-fall-through arm at `section/mod.rs:336` still emits
      the bare-line shape; the upstream early-validation arm
      shipped in this commit emits the grouped listing. The two
      should converge on a shared formatter.
- [ ] **Border preview "is one active?" surface** (API/UX HIGH):
      `border preview show` doesn't exist; staged previews are
      silently cancelled by any committing edit. A status verb
      would close the trap state.
- [ ] **Inconsistent "preset custom first" hint phrasing** (API/UX
      MEDIUM): `border` says "run \`border preset custom\` first";
      `section frame` says "Run \`section frame preset=custom\`
      first"; canvas says "run \`<label> preset custom\` first".
      Same gate, three phrasings — single shared formatter.
- [ ] **`fast_resize_start` LMB-release origin gate test** (touch
      deep-dive HIGH): the symmetric flag check on the right
      release arm is pinned (T2) but the left release arm's
      missing check (HI-1 above) has no negative test.
- [ ] **Layering inversion is wider than T4 admits** (architecture
      CRITICAL): 22 reach-up sites across `app/dispatch/cross_dispatch/`
      not just `style.rs` — `edges.rs` (8), `camera.rs` (2),
      `action_core.rs` (1). Update T4 follow-up to reflect the
      pattern, not the localised seam.
- [ ] **`parse_section_kv` byte-duplicated** (architecture HIGH):
      across `section/mod.rs:373-380` and `section/frame.rs:432-439`.
      §5 violation; should `use super::parse_section_kv`.
- [ ] **Three parallel `BorderPreviewTarget*` enums** (architecture
      HIGH): `keybinds::BorderPreviewTargetKind`,
      `document::BorderPreviewTarget`,
      `baumhard::scene_builder::BorderPreviewTargetRef<'a>`.
      The keybinds-side discriminator exists for `Hash + Eq`
      reasons; visible enum tax.
- [ ] **`Action` grew +29 variants** (architecture MEDIUM): the
      `SetSection*` family carries stringly-typed `f64` / `usize`
      payloads (e.g. `SetSectionOffsetDelta { dx: String, dy: String }`)
      because `Action` derives `Hash + Eq`. Typed payloads with a
      custom `Hash` impl would catch parse errors at construction
      rather than at dispatch.
- [ ] **`InitState` / `WasmInputState` god-struct creep**
      (architecture MEDIUM): InitState 22 → 25 fields;
      WasmInputState 10 → 12. The cross-platform mirror fields
      (`interaction_mode`, `touch_recognizer`) double the
      construct/borrow ceremony.
- [ ] **`canvas.rs::positional_subverb_to_edits` 165 LOC**
      (code-quality HIGH): three deeply-nested `match verb` arms
      with shared logic. Split per verb-class.
- [ ] **`_config` suffix rename inconsistency** (code-quality HIGH):
      T5 (whole-PR T6) renamed `set_canvas_default_border_config →
      set_canvas_default_border` cleanly, but
      `set_canvas_default_section_frame_border_config`,
      `set_section_frame_border_config`, and
      `set_node_border_config` keep the suffix. Either rename
      consistently or document why partial.
- [ ] **§5 dup of `parse_side_selector` / `parse_corner_selector`**
      (code-quality CRITICAL): reproduced inline in `canvas.rs:350-360,
      413-423` instead of using the `border/execute.rs` originals.
      The non-custom-preset gate is also duplicated.
- [ ] **§5 dup between `node_resize.rs` and `section_resize.rs`**
      (code-quality CRITICAL): differ on ~30 of 290 lines; same
      struct shape, same `ThrottledInteraction` impl, mostly
      identical drain/release. Generic-ify or share.
- [ ] **`expect()` in scene-build path** (code-quality CRITICAL):
      `lib/baumhard/src/mindmap/scene_builder/section_frame.rs:201`
      `expect("section_targeted implies preview is Some")`. Per-
      frame and §9 forbids panic; swap to `if let Some(...)`.
- [ ] **`expect()` in interactive click path** (code-quality
      MEDIUM): `app/click.rs:224, 234` `hit_section.expect("guarded
      above")`. Convert to `if let Some(idx)`.
- [ ] **Active-preview-rebuild bench missing** (performance
      MEDIUM): `cargo bench` covers the steady state but not the
      `build_node_elements` path with `border_preview = Some(...)`.
      Add a criterion bench.
- [ ] **`preview_node_ids` linear scan in `node_pass.rs`**
      (performance HIGH): O(N·K) per active preview. Hoist the
      target ids to a `HashSet<&str>` once per call.
- [ ] **`live_selection_node_ids` / `live_selection_section_pairs`
      clone allocations** (performance MEDIUM): the drift check
      allocates `Vec<String>` / `Vec<(String, usize)>` per frame
      while a preview is active. Borrowing variants would be a
      small win.
- [ ] **`do_*()` benchmark-reuse for `apply_view_to_slot`** (test
      coverage HIGH): hot path in `lib/baumhard/src/mindmap/border.rs`
      lacks the bench-reuse shape `TEST_CONVENTIONS §T6` calls for.
- [ ] **Touch recognizer → Action wiring untested end-to-end**
      (test coverage HIGH): three layers tested, the seams aren't.
- [ ] **`resize_mode_lifecycle.rs` is a flag round-trip** (test
      coverage HIGH): manually flips `mode` instead of dispatching
      the side effect — drops side-effect-consumer regressions.
- [ ] **Documentation drift in plan** (docs MEDIUM): plan Batch 5
      cites `section/mod.rs` as ~700 LoC (actually 2037);
      `predicates.rs:33` (actually `:35`); `document/mod.rs:520-523`
      gate cite (now spread across 88, 93, 516, 545, 608, 635);
      `CancelMode → ExitMode` rename marked deferred but actually
      shipped. Spot-fix or add a once-over follow-up.
- [ ] **`format/sections.md` 9-subverb table omits `frame`** (docs
      MEDIUM): table says "Nine subverbs" then renders 10 rows
      with no `frame` row.

Verification (post-Batch-8 ship): 2629 tests pass on
`./test.sh`; wasm32 cross-compile clean; `cargo doc --workspace
--no-deps` clean; `cargo bench` runs all Plan §7.4 benches
without regression.

### 8.3 Cross-batch dependencies

```
Batch 1 (Mode infra)
  ├── Batch 2 (Resize mode) — depends on InteractionMode::Resize
  ├── Batch 3 (NodeEdit visuals) — depends on InteractionMode::NodeEdit
  └── (Batch 4 fast-resize depends on Batch 2's mode infra
        but not on Batch 3's NodeEdit work)

Batch 2 ──── Batch 4 (Fast-resize) — depends on Resize-mode plumbing
Batch 3 ──── Batch 5 (Section verb) — depends on NodeEdit existing
Batch 5 ──── Batch 6 (Border verb) — independent, can run in parallel
                                      with 5 if same person isn't
                                      doing both
Batches 1-6 ──── Batch 7 (Touch parity) — depends on every prior
                                          mode existing
Batch 7 ──── Batch 8 (Polish) — depends on everything

```

Recommended order: 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8.

Batches 5 and 6 are independent and can be parallelised across two
sessions if available.

## 9. Open questions

These are deliberately unresolved — the implementing session decides
based on what feels right at the time. Each is small enough that
deferring to the implementer is fine.

1. **Should `mode resize` enter on a per-section target by default,
   or always go to node-resize when ambiguous?** The current spec
   (§6.2) says "node if Single, section if Section". But what about
   a `Section` selection where the user intends to resize the
   *node*? Possible answer: a `[target=node|section]` kv on `mode
   resize` to override.

2. **Should `border preview` allow staging multiple verbs?** E.g.
   `border preview preset=heavy` then `border preview color=#fff`
   — each writes into the same preview, accumulating. Or are
   subsequent previews replacements? Current spec is silent;
   accumulation feels more useful but replacement is simpler.

3. **`Action::EnterNodeEdit` on a `Multi(ids)` selection — what is
   the right behavior?** Current spec says no-op + log. An
   alternative is "enter NodeEdit on the first id"; another is
   "enter NodeEdit on each id sequentially via macro". Decide on
   first implementer encounter.

4. **`EnterNodeEdit` short-circuit on single-section nodes — should
   it skip NodeEdit entirely, or pass through it for a frame?** The
   spec says "in one transition". If the user has a macro like
   `[EnterNodeEdit, EnterSectionEdit]` and the node is single-
   section, does step 2 fire on a node already in SectionEdit? It
   should be a no-op, but worth pinning.

5. **Where does the status-bar GlyphArea sit on the canvas?** Top-
   center? Top-left? Configurable? For now, top-center; revisit if
   it conflicts with anything else in the overlay layer (FPS
   overlay sits in top-right today).

6. **Should `border preview` support live-preview during keybind /
   macro chains?** I.e. binding `Ctrl+1` to `border preset=light;
   border preview commit` — does the preview render for one frame
   between the two? For now: yes, but it's a single-frame flicker
   and probably not user-noticeable. If it bugs anyone, the macro
   can be one Action `BorderApplyPreset(name)` that combines preset
   + commit.

7. **Touch long-press tolerance.** 350ms is the spec. Native has
   different defaults than WASM. Tune empirically.

## 10. Out of scope

These are explicitly NOT in this overhaul. They sit in the named
trajectory but belong in their own work.

- **Shape-aware borders** for ellipses and other non-rectangular
  shapes. Today's `format/enums.md:54-57` notes the silent-drop;
  fixing it is a renderer pipeline change, larger than UX.
- **GUI buttons / toolbar / palette UI.** The mode infrastructure
  is designed so a GUI attaches without further refactoring (every
  user-facing operation is an `Action`), but the GUI itself is
  separate.
- **Per-section chrome (background, frame, padding) in the data
  model.** Sections are deliberately chrome-less per
  `format/sections.md`. The redesign respects this.
- **Multi-node bulk resize.** "Resize 5 selected nodes uniformly"
  is a clean future extension via `MouseGesture::TwoFingerDrag`
  applied to a `Multi` selection — but not in this batch.
- **Section reordering** (drag a section to swap position N ↔ M).
  Useful but distinct from add/delete/split.
- **Section-level palette / theme variant binding.** Sections lack
  chrome; palette resolution rides on the parent node.
- **Cosmic-text shaping changes.** All text rendering stays in
  current shape.
- **Keybind config GUI.** User edits `keybinds.json` by hand for now.
- **Border presets beyond the existing five.** Adding `dashed`,
  `dotted`, `thick`, etc. is a separate batch.
- **Macro reach extension.** The new Actions inherit existing
  macro-tier privilege gates; no new tier semantics.
- **Schema migration.** Plan target: all format schemas
  (`MindNode`, `MindSection`, `GlyphBorderConfig`, `Canvas`)
  unchanged. **As-shipped divergence**: three new optional
  fields landed (Plan-Adherence reviewer flagged):
  `MindSection.frame_border: Option<GlyphBorderConfig>`,
  `Canvas.default_section_frame_border: Option<GlyphBorderConfig>`,
  `Canvas.default_focused_section_frame_border: Option<GlyphBorderConfig>`.
  All default to `None`, so legacy `.mindmap.json` files load
  unchanged (serde defaults absorb absent fields). The fields
  carry the per-section + canvas-default frame-border style
  cascade Batch 2/5 ship; without them the section-frame chrome
  has no model anchor. Honored "no breaking change" in spirit
  (legacy maps load) but not in letter; Batch 8 should formalize
  these in `format/sections.md` / `format/canvas.md` and add a
  schema-version bump if the migration story warrants one.

## 11. Critical files (one-stop reference)

Files most heavily modified across batches — keep these open during
work.

- `src/application/app/interaction_mode.rs` (NEW — Batch 1)
- `src/application/app/mod.rs` (Batches 1, 2)
- `src/application/app/run_native.rs`, `run_native_init.rs`,
  `run_wasm/mod.rs` (Batch 1)
- `src/application/app/click.rs` (Batches 1, 3)
- `src/application/app/event_mouse_click.rs` (Batches 2, 3, 4)
- `src/application/app/event_cursor_moved.rs` (Batches 2, 3, 4)
- `src/application/app/event_keyboard.rs` (Batch 3)
- `src/application/app/input_context.rs`, `input_context_core.rs`
  (Batch 1)
- `src/application/app/dispatch/native.rs`,
  `dispatch/action_core.rs`, `dispatch/cross_dispatch/*` (Batches 1-6)
- `src/application/app/text_edit/editor.rs` (Batch 3)
- `src/application/app/touch_gesture.rs` (NEW — Batch 7)
- `src/application/app/scene_rebuild.rs` (Batches 2, 3)
- `src/application/scene_host.rs` (Batch 3 — status bar)
- `src/application/keybinds/action/mod.rs` (Batches 1-6)
- `src/application/keybinds/bind.rs` (Batches 4, 7)
- `src/application/keybinds/config.rs` (Batches 1-7)
- `src/application/keybinds/context.rs` (Batch 3)
- `src/application/console/commands/border/*` (Batch 6)
- `src/application/console/commands/section/*` (Batch 5)
- `src/application/console/commands/mod.rs` (Batches 5, 6)
- `src/application/console/commands/node.rs` (Batches 2, 4)
- `src/application/console/commands/canvas.rs` (NEW — Batch 6)
- `src/application/console/predicates.rs` (Batches 5, 6 —
  `node_or_section_selected`)
- `src/application/document/mod.rs` (Batch 2 — the one-line gate)
- `src/application/document/types.rs` (preserved as-is, but tests
  expand)
- `src/application/document/nodes/mod.rs` (Batch 5 — add/delete/split
  setters)
- `src/application/document/nodes/border.rs` (Batch 6 — auto-promote
  removal)
- `src/application/document/hit_test.rs` (Batches 2, 4)
- `lib/baumhard/src/mindmap/scene_builder/section_resize_handle.rs`
  (Batches 2, 4)
- `lib/baumhard/src/mindmap/scene_builder/section_frame.rs` (NEW —
  Batch 3)
- `lib/baumhard/src/mindmap/scene_builder/builder.rs` (Batches 2, 3,
  6)
- `lib/baumhard/src/mindmap/scene_builder/mod.rs` (Batches 2, 3, 6
  — `SceneSelectionContext` extensions)
- `lib/baumhard/src/mindmap/scene_builder/node_pass.rs` (Batch 3 —
  dimming)
- `format/sections.md` (Batches 5, 8)
- `format/border-patterns.md` (Batches 6, 8)
- `CONCEPTS.md` (Batch 8)
- `CLAUDE.md` (Batch 8 if needed)

## 12. Verification recipe (per batch and end-to-end)

After every batch:

1. `./test.sh` — passes; new test count modestly increases per the
   per-batch tasks.
2. `./test.sh --lint` — `cargo fmt --check` clean, `cargo clippy`
   clean. Treat new warnings as findings.
3. `./test.sh` also type-checks `wasm32-unknown-unknown`; cross-
   platform drift fails the run. Critical for Batches 1, 3, 7.
4. `./build.sh` — both native and WASM artifacts build cleanly.
5. `./test.sh --bench` for Batches 2, 3 in particular (handle gating
   path, scene-builder section-frame pass).
6. Manual smoke: `./run.sh maps/testament.mindmap.json` on native
   AND WASM, walk through the relevant integration scenarios from
   §7.2.
7. `maptool verify` against every map in `maps/` — structural sanity
   that no on-disk format change slipped in. None should change in
   any batch (this whole plan is data-model-stable).
8. `./test.sh --coverage` after Batch 8 — confirm no coverage
   regressions in critical paths (dispatch, mode transitions, scene
   build).

## 13. Glossary

- **`Action`**: An entry in the `Action` enum, the discrete user-
  facing operation that the dispatch funnel routes (CODE_CONVENTIONS §3).
- **`InteractionMode`**: The new cross-platform enum carrying the
  active high-level interaction mode (§3.1).
- **`SelectionState`**: The existing enum carrying *what* is
  selected (`document/types.rs:158-206`); orthogonal to mode.
- **`DragState`**: The existing enum tracking the per-press drag
  state machine (`app/mod.rs:386-461`).
- **`ResizeTarget`**: A small enum identifying the target of Resize
  mode (Node or Section).
- **`MouseGesture`**: The existing enum identifying mouse gestures
  for keybind dispatch (`keybinds/bind.rs:44-66`).
- **`MindNode`**: The data-model node (`baumhard/.../model/node.rs:38`).
- **`MindSection`**: A positioned text-bearing surface inside a
  `MindNode` (`baumhard/.../model/node.rs:278-342`).
- **`GlyphBorderConfig`**: The per-node border configuration
  (`baumhard/.../model/node.rs:446-482`).
- **`SceneSelectionContext`**: The struct passed into the scene
  builder carrying selection / mode / preview overrides
  (`baumhard/.../scene_builder/mod.rs`).
- **`Throttled(NodeResize)` / `Throttled(SectionResize)`**: Variants
  of `DragState::Throttled(ThrottledDrag)` carrying the in-flight
  resize gesture state.
- **Single-section node**: A node with `sections.len() == 1`. The
  hit-test fold `hit_test_target` returns `NodeContainer` for
  these (`document/hit_test.rs:130-138`); their UX collapses to
  whole-node throughout the redesign.
- **Multi-section node**: A node with `sections.len() >= 2`. The
  redesign gives these the explicit NodeEdit mode for section-aware
  authoring.
- **Fill-parent section**: A `MindSection` with `size: None`,
  rendering at the parent node's full AABB.
- **Anchored section**: A `MindSection` with `size: Some(Size)`,
  rendered at an explicit AABB inside the parent. Resize handles
  emit only for anchored sections.
- **Default mode**: `InteractionMode::Default`. Today's normal
  state; selection drives all UX, no mode-gated chrome.
- **NodeEdit mode**: `InteractionMode::NodeEdit { node_id }`. The
  active node is treated as a mini-canvas; section frames render;
  section clicks set Section selection.
- **SectionEdit**: The implicit state when `TextEditState::Open`
  inside `NodeEdit`. Cosmic-text caret active.
- **Resize mode**: `InteractionMode::Resize { target }`. Resize
  anchors visible on the target.
- **Fast-resize**: The `Ctrl+RightDrag` gesture that resizes from an
  anchor inferred by cursor quadrant, without entering Resize mode.
