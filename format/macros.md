# Macros

Macros are named, reusable sequences of user-driven actions that the
keybind layer can fire as a single unit. A macro is a list of
[steps](#step-kinds), executed in order; bind a key combo to a macro
id in `keybinds.json` and pressing the key runs the whole sequence.

Macros are a *user-authoring layer* on top of the existing dispatch
funnel — they don't introduce new behaviours, just orchestrate
existing ones (`Action`, `CustomMutation`, console verbs).

This document is the on-disk format reference. For the Rust-side
types see `src/application/macros/`; the dispatcher lives in
`src/application/app/dispatch/macro_core.rs::dispatch_macro`
(cross-platform, abstracted over the `MacroDispatchTarget` trait —
the native shim that wraps `InputHandlerContext` is at
`src/application/app/dispatch/native.rs::dispatch_macro`).

## Where macros come from

Four loader tiers contribute to the active registry, in ascending
precedence (later writers override earlier ones with the same `id`):

<!-- SOURCE-OF-TRUTH: the tier order below is also encoded in
     five other places that must move together when the order or
     set of tiers changes:
       1. src/application/source_tier.rs — the SourceTier enum
          variant order, shared with the custom-mutation registry
          and pinned by that module's tests.
       2. src/application/app/run_native_init.rs::build — the load
          order of the App + User tier calls AND the
          rebuild_map_macros + rebuild_inline_macros calls.
       3. src/application/app/console_input/exec.rs — the
          rebuild_map_macros + rebuild_inline_macros invocations
          in the document-replace path. Inline rebuilds AFTER Map
          so Inline's higher precedence wins on id collision.
       4. src/application/macros/loader/ — the parse_*_macros /
          rebuild_*_macros helpers themselves.
       5. format/ipc.md §"Trust model" + work_plans/LLM_IPC.md §D4
          — the IPC surface is pinned to the User tier (it is
          deliberately NOT a tier of its own); adding or reordering
          tiers must re-evaluate that mapping.
     Update all six in the same commit. -->

1. **Application bundle** — `assets/macros/application.json`,
   compiled into the binary via `include_str!`. Lowest precedence so
   users can customize anything shipped by the app. Tier:
   `SourceTier::App`. Cross-platform (native + WASM).
2. **User file** — `$XDG_CONFIG_HOME/mandala/macros.json` on native
   (falls back to `$HOME/.config/mandala/macros.json`). On WASM,
   `?macros=<urlencoded-json>` query param > `localStorage` under
   the `mandala_macros` key > empty. Both targets parse through
   the shared `loader::parse_user_macros_json`. Tier:
   `SourceTier::User`.
3. **Map-inline** — `MindMap.macros` on the loaded document.
   Refreshed at initial load on both targets, plus every `open` /
   `new` console verb on native. Tier: `SourceTier::Map`.
4. **Node-inline** — `MindNode.inline_macros` on individual nodes.
   Loaded alongside Map tier (same trigger sites). Highest
   precedence — overrides Map / User / App on id collisions.
   Authors should namespace ids (e.g. `"node-id.action"`) to
   avoid collisions across nodes since the registry is
   id-keyed flat. Tier: `SourceTier::Inline`. Cross-platform.

The `SourceTier` of a macro is **loader-pinned** — assigned at the
loader call site, never read from the on-disk content. A user
editing `~/.config/mandala/macros.json` cannot smuggle `App` tier
into their entries.

### Within-tier and cross-tier collision semantics

Two entries with the same `id` in the same tier (e.g. a Map-tier
`macros` array containing two entries both `id: "x"`) follow
last-writer-wins — the second entry overwrites the first.
Authoring tip: keep ids unique within a single source.

**Inline tier is special.** Inline macros are scoped per-node,
but the registry is flat (id-keyed across all nodes). So
"within-tier" for Inline means "across every node in the
document." And because `MindMap.nodes` is a `HashMap`, the
walk order is non-deterministic — the "winner" for an id
duplicated across nodes varies per process start. The loader
emits a `warn!` on cross-node Inline collisions, but the only
robust fix is namespacing: prefix each inline-macro id with the
owning node's id (e.g. `"3.2.1.save-and-quit"`).

Cross-tier: a higher-tier entry **shadows** a lower-tier entry
with the same id — both coexist in their own tier slots, with
lookup walking high-to-low precedence and returning the first
hit. Higher tiers take precedence on lookup, but the lower-tier
entry is preserved underneath:

- Open document A with `Map`-tier `id: "save-and-quit"`. Lookup
  returns the Map version while document A is open.
- Open document B with no `macros` → `clear_tier(Map)` runs,
  removing only the Map slot. The User-tier entry **re-emerges**
  on the next lookup — shadowing is reversible.

Within-tier last-writer-wins still applies (two entries with
the same id in the same tier — the second wins). Authors
should still namespace ids defensively when targeting the
Inline tier specifically, since cross-node Inline collisions
have non-deterministic winners (HashMap iteration order); see
the Inline-tier note above.

## Privilege model — read this before shipping a non-User loader

The dispatcher gates two surfaces on the macro's `SourceTier`:

- **`MacroStep::ConsoleLine`** runs an arbitrary console verb
  (`save`, `open`, `mutation apply`, kv-shaped style setters, etc.).
  User-tier-only via `SourceTier::allows_console_line`.
- **Destructive / I/O / clipboard `Action` variants**
  (`SaveDocument`, `DeleteSelection`, `Cut`, `Paste`, `Copy`,
  `OrphanSelection`, `CreateOrphanNode`, `CreateOrphanNodeAndEdit`,
  `NewDocument`) are User-tier-only via
  `SourceTier::allows_action`. Navigation / view-state Actions
  (zoom, pan, selection traversal, undo, etc.) pass freely.

Privilege rejections **fail-closed** — the dispatcher aborts the
rest of the macro on a rejected step so a hostile pattern like
`[DeleteSelection, ConsoleLine(rejected), SaveDocument]` cannot
sneak its outer steps past the gate. Honest mistakes (unbound
action, no document loaded) keep the existing best-effort
`continue` semantic.

**IPC is not a tier.** Commands arriving over the `--ipc` socket
([`format/ipc.md`](./ipc.md)) execute at `User` posture — the user
owns the flag exactly as they own this file. Macros fired *via* IPC
(`act.macro`) keep their own loader-pinned tier: IPC initiates, it
never escalates, and the fail-closed gates above run unchanged.
Rationale: `work_plans/LLM_IPC.md` §D4.

### Threat model

Map and Inline tiers ship today; **opening any `.mindmap.json` from
an untrusted source IS a privilege event**. Treat third-party
mindmap files as code, not data:

- A hostile mindmap can bind any non-destructive Action to a hotkey
  the user is likely to press (Enter, Tab, etc.).
- A hostile mindmap can carry CustomMutation steps that mutate
  document state in surprising ways (the changes are in-memory and
  undoable; the worst case is an annoying user experience, not data
  exfiltration).
- A hostile mindmap CANNOT run console verbs, save the file, delete
  selection, or touch the clipboard — those are all gated above.

If a future contributor adds a `DocumentAction` variant that
performs file I/O, network access, or arbitrary content load, the
gate must extend to cover it. `DocumentAction` is `#[non_exhaustive]`
specifically to surface this when the variant is added — see
`lib/baumhard/src/mindmap/custom_mutation/mod.rs`.

## On-disk format

Macros are loaded as a top-level JSON array of macro objects.
Application-bundle, user-file, and map-inline tiers all use this
same shape; tier is assigned by the loader, not the file.

```json
[
  {
    "id": "save-and-zoom-out",
    "name": "Save and Zoom Out",
    "description": "Persist the current map and back off the camera.",
    "steps": [
      { "kind": "Action", "action": "SaveDocument" },
      { "kind": "Action", "action": "ZoomOut" }
    ]
  },
  {
    "id": "tag-as-inbox",
    "name": "Tag as Inbox",
    "steps": [
      { "kind": "CustomMutation", "id": "set-tag-inbox" },
      { "kind": "ConsoleLine", "line": "save" }
    ]
  }
]
```

### `Macro` fields

| field | type | notes |
|---|---|---|
| `id` | `string` | Required. Lookup key. Higher-tier macros override lower-tier ones with the same id. |
| `name` | `string` | Optional, defaults to `""`. Display label for future macro pickers / inspection. |
| `description` | `string` | Optional, defaults to `""`. Human-readable explanation for the same. |
| `steps` | `[MacroStep]` | Required. The ordered sequence executed when the macro fires. |

### Step kinds

Each step is an object with a `"kind"` discriminator (internally
tagged) plus the kind-specific fields.

#### `Action`

Run a built-in `Action` against the current `InputHandlerContext`.
Routes through the same `dispatch_action` that keyboard / mouse
input use.

```json
{ "kind": "Action", "action": "SelectAll" }
```

The `action` value is the variant name from
`src/application/keybinds/action/mod.rs`. Examples: `"Undo"`,
`"SaveDocument"`, `"ZoomReset"`, `"SelectAll"`,
`"DoubleClickActivate"`. Modal-context actions (`TextEditCursorLeft`,
`PickerCommit`, `ConsoleSubmit` etc.) are accepted but only fire
when the matching modal is open.

Some Actions are gated for non-User tiers — see
[Privilege model](#privilege-model--read-this-before-shipping-a-non-user-loader).

#### `CustomMutation`

Apply a custom mutation by id, against a resolvable target node.
Routes through the same path keybind-triggered custom mutations
use (animation-aware via `cm.timing`, always invokes
`apply_document_actions`).

```json
{ "kind": "CustomMutation", "id": "nudge-right" }
```

```json
{
  "kind": "CustomMutation",
  "id": "highlight-trunk",
  "target": { "node_id": "1.2.3" }
}
```

| field | type | notes |
|---|---|---|
| `id` | `string` | Required. Must resolve in the document's `mutation_registry`; unknown ids log a `warn!` and the step is skipped. |
| `target` | `MacroTarget` | Optional, defaults to `"current_selection"`. |

`MacroTarget` is one of:
- `"current_selection"` — use the currently-selected single node.
  Multi / edge / portal selections cause the step to skip.
- `{ "node_id": "..." }` — always target the named node. The
  dispatcher checks the id exists in the document; missing ids
  log a `warn!` and the step is skipped.

#### `ConsoleLine`

**User-tier-only.** Re-parse the given line through the console
parser and run it as if typed. Lets macros leverage every
parameterised verb (`border preset=triple`, `color bg=#fafafa`,
etc.) without needing a bespoke step kind.

```json
{ "kind": "ConsoleLine", "line": "save" }
```

ConsoleLine requires a loaded document. Macros fired before any
document loads silently skip the step with a `warn!` — bind paths
through CLI args / `?map=` query rather than a startup-hotkey
macro.

A non-User-tier macro that contains `ConsoleLine` steps will be
**rejected entirely** (fail-closed); the dispatcher logs a `warn!`
and aborts the macro after the first rejected step.

##### ConsoleLine on WASM

WASM has no console runtime — the `console_input` module is
`#[cfg(not(target_arch = "wasm32"))]`-gated, so there's no
`execute_console_line` to call. **User-tier ConsoleLine steps on
WASM log `warn!` and skip; the macro continues with the next
step** (fail-soft, NOT abort). This matches the User-tier "step
failed" posture used elsewhere (e.g. unknown CustomMutation id),
and prevents a copy-pasted-from-desktop macro shaped
`[Action::ZoomIn, ConsoleLine("save"), Action::ZoomIn]` from
fail-closed-aborting at step 2 and never running step 3.

The privilege gate above is unchanged — non-User tiers
(`App` / `Map` / `Inline`) still fail-closed-abort the macro on
the FIRST ConsoleLine encounter, identical to native. Only the
User-tier branch differs in *what* execution looks like
(log-skip on WASM, real exec on native).

## Resolution order at dispatch

When a key combo fires, the keyboard handler walks three resolution
tiers in order:

1. `keybinds.action_for_context(...)` — built-in `Action` variants.
2. `keybinds.macro_for(...)` — user-defined macros from
   `macro_bindings`. Unknown macro ids fall through to tier 3.
3. `keybinds.custom_mutation_for(...)` — per-node custom mutations
   from `custom_mutation_bindings`.

A built-in `Action` wins over a colliding macro binding. To
override a built-in Action with a macro, first unbind the Action's
keybind:

```json
{
  "copy": [],
  "macro_bindings": { "Ctrl+C": "my-extended-copy" }
}
```

## Loader resilience

Each loader is best-effort. Failures (missing file, malformed
JSON, unknown step kinds, invalid action names) log a `warn!` and
the loader returns an empty / partial slice. The application boots
even if the macros file is broken. The application bundle is a
build-time invariant and is parsed with `expect()` — a malformed
bundle is a startup-time bug, not a user input error.
