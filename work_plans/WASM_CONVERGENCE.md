# WASM Convergence

This document is the porting guide for unifying the WASM and native
input pipelines. Mandala targets both platforms as first-class deployments
(per `CODE_CONVENTIONS.md §4`); today's WASM target has a curated subset
of the modals, gestures, and Actions that native ships.

If you're picking this work up, **start here, then read in order**:
[`CONCEPTS.md §5 "Action dispatch"`](../CONCEPTS.md), the
`Action::wasm_compatibility` method in
[`src/application/keybinds/action/mod.rs`](../src/application/keybinds/action/mod.rs),
[`src/application/app/dispatch/native.rs`](../src/application/app/dispatch/native.rs)
(the reference implementation), and
[`src/application/app/run_wasm/`](./src/application/app/run_wasm/).

## The current shape

**Native** (`src/application/app/`) has:
- `dispatch_action(action, &mut InputHandlerContext, hit)` — the
  single funnel every Action body runs through.
- `dispatch_macro(macro_id, ctx)` — same shape for macros.
- `dispatch_custom_mutation_for_key` — same shape for keybind-
  triggered custom mutations.
- A 21-field `InputHandlerContext` covering every modal /
  state-machine / per-frame field the dispatch arms might touch.
- A `MacroRegistry` with App / User / Map / Inline tiers loaded.

**WASM** (`src/application/app/run_wasm/`) has:
- Its own `WasmInputState` struct holding the cross-platform fields
  plus a `MacroRegistry`.
- An inline `match action { ... }` block for keyboard input where
  every Compatible Action arm calls into the shared
  `dispatch::cross_dispatch` helper module. The keyboard chain runs
  all three tiers — Action → Macro → CustomMutation — the same as
  native.
- No double-click ladder. **Closed by #29**: the `match &click_hit`
  ladder that used to be "the largest remaining Track-A duplication"
  is gone from `run_wasm/`. Both targets resolve
  `MouseGesture::DoubleClick` through the keybind table and run one
  body in `dispatch::cross_dispatch::pointer`.

Tracks B (macro registry) and C (full context-type unification)
landed; both targets dispatch every Compatible Action through the
same `dispatch::action_core::dispatch_compatible` function over an
`InputContextCore<'a>` view. WASM has a `MacroRegistry` populated
at startup with the same loader/parser code paths native uses.
Track A.3 — lifting per-arm bodies into `dispatch::cross_dispatch` —
is the recommended path for any new Compatible Action.

## The convergence target

Long-term: a single `dispatch_action` callable from both targets,
with WASM gradually gaining the missing systems so more Actions
flip from `NativeOnly` to `Compatible`.

The `Action::wasm_compatibility(&self) -> WasmCompatibility` method
([`src/application/keybinds/action/mod.rs`](../src/application/keybinds/action/mod.rs))
is the typed API surface. It classifies every Action as
`Compatible` or `NativeOnly`. The match is exhaustive on `Action`
(an open enum via `#[non_exhaustive]`); a developer adding a new
variant is forced by the compiler to make the call.

`WasmCompatibility::Compatible` means "this Action's body only
touches state that exists on both targets" (`MindMapDocument`,
`Renderer`, `text_edit_state`, mouse gestures, the macro registry).
`WasmCompatibility::NativeOnly` means "this Action's body needs
native-only state" (console, color picker, label editor, AppMode,
DragState, filesystem). The doc-comment on `wasm_compatibility`
spells out the rules in detail.

## Track A — port an Action to WASM

When you want a specific feature in the browser, or you've added
a new Compatible variant and need to wire it through both
dispatchers. **Three paths**, in order of preference:

- **Path A.3 — partial Track C (preferred for Compatible Actions).**
  Add a per-action helper to `dispatch/cross_dispatch.rs` that takes
  the typed payload + a `RebuildContext`. Both dispatchers call the
  same function — no mirror tax. This is what every camera /
  selection / FPS / parametric Compatible arm does today (see
  the `apply_zoom_step`, `apply_select_all`, `apply_set_color_axis`
  shapes for templates).
- **Path A.1 — inline mirror arms.** For Compatible Actions whose
  bodies need state only one side has, OR for NativeOnly Actions
  you want partial WASM coverage of, add an inline arm to
  `run_wasm/` that touches WASM-shaped state. Dispatch logic
  duplicates until A.3 lift consolidates.
- **Path A.2 — full Track D.** Once both targets share `&mut self`
  shape on event handlers, route through `dispatch_action` directly.
  Cleanest endpoint, biggest refactor. See "Per-arm event-handler
  shape divergence" below for the open question.

### Steps for Path A.1

1. Decide whether to port the underlying system (full console on
   WASM) or surface a WASM-shaped equivalent (e.g. a `<dialog>`
   element instead of an in-canvas overlay).
2. Add the corresponding state to `WasmInputState`.
3. Open the matching native dispatch arm in
   `src/application/app/dispatch/native.rs` to understand the body.
4. Write a parallel arm in `run_wasm/`'s `match a { ... }` block
   that does the same thing against `WasmInputState`. Comment with
   `// MIRROR OF dispatch/native.rs::Action::Foo arm — keep in
   sync until A.3 lift consolidates.`
5. Flip the Action's `wasm_compatibility` classification to
   `Compatible`. Update the corresponding test in
   `src/application/keybinds/tests.rs`.

## Track-D meta — keep the privilege model intact

The macro privilege gate (`SourceTier::allows_console_line`,
`allows_action`, fail-closed in `dispatch_macro`) MUST remain
single-sourced on both targets. The
[`format/macros.md`](../format/macros.md) "Privilege model"
section is the authoritative spec; the implementation lives in
`src/application/macros/mod.rs` and the enforcement loop in
`src/application/app/dispatch/macro_core.rs`. The
`WasmCompatibility` classification is orthogonal — a
`Compatible` Action might still be denylisted by
`SourceTier::allows_action` for non-User macros (e.g.
`Action::SaveDocument` would be `Compatible` once WASM gains a
save path, but it'd still be in the denylist because hostile
mindmaps shouldn't invoke it).

`SourceTier::allows_action` and `allows_console_line` live in
`src/application/macros/mod.rs` (cross-platform). The fail-closed
enforcement loop is in `dispatch::macro_core::dispatch_macro`,
abstracted over a `MacroDispatchTarget` trait so native and WASM
share the body byte-for-byte.

Re-implementing the privilege check inline is **forbidden** —
it's the threat-model defence and must be single-sourced. A
forked enforcement copy would silently drift when a future
contributor adds an Action to the denylist.

## What's deferred today (and tracked in TODO.md)

- The inline label / portal-text editors and the color picker on
  WASM. Track A on individual Actions.
- The console on WASM. Track A.
- `AppMode` (Reparent / Connect) on WASM. Track A.
- `DragState` / continuous-drag gestures (`PanCanvas`) on WASM.
  Track A — note that WASM has its own `pending_click` mechanism
  that may serve as the basis.
- Filesystem on WASM (`OpenDocument` / `SaveDocumentAs` /
  `NewDocumentAt` parametric Action variants stay `NativeOnly`
  pending a chosen storage strategy).
- IME / Focused input event arms — the catch-all in
  `WasmApp::handle_window_event` documents these by name; each
  needs its own `event_*.rs` sibling once wired. IME is required
  for non-Latin text editing in the inline node-text editor.
  (Touch left this list when `run_wasm/event_touch.rs` landed; its
  recognizer driving is shared as of #29 — see below.)
- Maptool migration on WASM (`maptool convert --sections` is
  native-only by construction; a browser-only authoring flow
  that loads a legacy map needs an in-app migration path).

## Closed by #29 — input-path duplication

These behaviors had a body on each target. Each now has exactly one,
in `dispatch::cross_dispatch` or alongside it, called from both. Grep
for the helper name to find both call sites.

| Behavior | Shared body |
| --- | --- |
| Double-click routing (node / portal / edge-label / empty) | `cross_dispatch::pointer::resolve_double_click_route` + `apply_double_click_activate` |
| Create-orphan-and-edit (was 3 copies) | `cross_dispatch::apply_create_orphan_node_and_edit` |
| Text-edit click-outside containment | `text_edit::release_stays_inside_edited_node` |
| Already-editing guard | `app::already_editing_same_target` |
| Wheel-delta decomposition | `app::wheel_lines` + `app::wheel_gesture` |
| Touch ingest → tick → dispatch | `cross_dispatch::pointer::drive_touch_event` + `touch_phase` |
| Load-time canvas warm | `scene_rebuild::warm_scene_at_load` |
| Macro-registry build | `macros::loader::build_macro_registry` |
| Camera-geometry reprojection | `scene_rebuild::rebuild_camera_geometry` |

Three funnel gaps closed with them:

- **Double-click now consults the keybind table on WASM.** It used to
  hardcode the behavior, so rebinding or unbinding
  `double_click_activate` was silently ignored in the browser.
- **Wheel zoom goes through the funnel on WASM.** It used to hardcode
  `factor = 1.1` and emit `CameraZoom` directly, bypassing
  `action_for_gesture` entirely. The post-zoom rebuild converges on
  native's narrow set too: base WASM ran `rebuild_scene_only` (all
  seven canvas roles) and now runs `rebuild_camera_geometry` (three).
  The four dropped roles — borders, section frames, and both
  resize-handle trees — are canvas-space and zoom-independent, so a
  zoom cannot move them; §4's mobile budget is the reason to converge
  on the narrow set rather than the wide one. Two consequences the
  call site spells out: `CameraPan` does not set
  `connection_geometry_dirty` (only `CameraZoom` does), so a wheel
  rebound to a pan Action reprojects nothing — identically to native;
  and there is no `!is_moving_node` term in the browser's guard,
  which is safe only while `WasmInputState` carries no `drag_state`.
- **The keyboard chain reaches the custom-mutation tier on WASM.** It
  stopped at Macro, so a `custom_mutation_bindings` entry worked on
  the desktop and was dead on the web. **Instant mutations only** —
  an entry with `timing.duration_ms > 0` takes
  `apply_keybind_custom_mutation`'s `start_animation` branch, which
  only queues the envelope. `drain_animation_tick` is the sole
  advance site and `drain_frame.rs` is native-gated, so a browser
  animation starts and never ticks. Pre-existing on the click-trigger
  path; the keystroke tier widens it. Registered in CLAUDE.md's
  "Dual-target status"; parity is a browser drain hung off the
  existing rAF render loop, pumping the same body.

`DoubleClickActivate` stays `wasm = NativeOnly`: its `EdgeLabel`
branch still reaches the single-line editor, and the "ANY NativeOnly
branch" rule classifies on that. Flipping it to `Compatible` waits on
the browser gaining a single-line editor.

It is a member of the **mixed-branch set**, which is now one list —
`keybinds::action::MIXED_BRANCH_ACTIONS` — read by both
`lift_mixed_branch_for_wasm_macro` and
`keybinds::tests::test_wasm_compatibility_mixed_branch_actions_are_native_only`.
Written out twice, the two drifted: the test named three members and
the lift named four. They could not simply be merged, either, because
the members do **not** share a classification — `ExitMode` is
mixed-branch and `Compatible`, since its native leftover is a step
(the `hovered_node` clear) rather than a branch reaching native-only
state. The list therefore carries the expected `WasmCompatibility`
alongside each member: the test asserts it per member instead of
asserting a blanket `NativeOnly`, and the lift's `debug_assert` fires
in any test build for a member with no verdict arm. Adding a member
now obliges both consumers instead of silently satisfying neither.

## Per-arm event-handler shape divergence

Native handlers are free functions taking `&mut
InputHandlerContext<'_>`. WASM handlers are inherent methods on
`WasmApp` because the `Rc<RefCell<Option<…>>>` cell projection
forces a `&mut self` shape. Track D's full convergence will
need to either remove the cells (convert `WasmApp` to own its
state directly) or accept the method shape on both sides. New
WASM event handlers added before Track D should follow the
method pattern.

## Smoke-testing the boundary

When you flip an Action from `NativeOnly` to `Compatible`, the
test in `src/application/keybinds/tests.rs` starts failing —
that's the signal to update the test alongside the classification.
The existing test suite covers the dispatch arm via the native
path; the WASM target has no headless harness
(`TEST_CONVENTIONS.md §T9`) — manual smoke via `trunk serve`.
