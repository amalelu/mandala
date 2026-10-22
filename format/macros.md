# Macros

Macros are named, reusable sequences of user-driven actions that the
keybind layer can fire as a single unit. A macro is a list of
[steps](#step-kinds), executed in order; bind a key combo to a macro
id in `keybinds.json` and pressing the key runs the whole sequence.

Macros are a *user-authoring layer* on top of the existing dispatch
funnel — they don't introduce new behaviors, just orchestrate
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
(`act.macro` with a `macro_id`) keep their own loader-pinned tier:
IPC initiates, it never escalates, and the fail-closed gates above
run unchanged. An anonymous step list sent as `act.macro`'s `steps`
was authored by the IPC client, so it runs at `User`. It runs
through the same gated step loop, not a second one. A gate
rejection is logged under the code `macro_gate_rejected`, which an
IPC reply carries as a warning. Rationale: `work_plans/LLM_IPC.md`
§D4.

### The load-time trust boundary

The privilege gates above answer "what may a map *do*". They are
only half the question, because a map does not need a capability to
kill the editor — it only needs a number the renderer trusts. Every
`f32` in the format ends up in something that is not defensive:
cosmic-text asserts a line height is non-zero, `f32::clamp` panics
on inverted bounds, and the border and connection builders size a
`String` and a `Vec` from authored geometry. None of those degrade;
an `assert!` inside the scene build and an allocator abort are not
panics, so CODE_CONVENTIONS §9's "log and keep running" has nothing
to run.

So the numbers are screened once, at the door, by
`lib/baumhard/src/mindmap/model/validate.rs`, which the loader runs
as its last invariant. It **rejects rather than repairs** — the same
posture the loader already took for structure — and the message
names the part to fix. What it covers:

- **Font metrics** — every `font_size_pt`, `size_pt`, and min/max
  pair on borders, connections, labels, portals, and text runs must
  be finite and within `[0.5, 4096]`pt. Zero is what aborts the
  shaper; a sub-point size asks the border fitter for millions of
  glyphs.
- **Inverted clamps** — a `min > max` pair is refused, because the
  size cascades hand one straight to `f32::clamp`.
- **Geometry** — node sizes, positions, section offsets, and Bezier
  control points must be finite and bounded. JSON has no `Infinity`
  literal, but `1e39` does not fit an `f32` and arrives as one.
- **Text runs** — sorted, non-overlapping, non-empty, inside the
  section's text. These were always the model's assumption;
  overlapping runs made the styled-span bridge re-emit the same
  graphemes once per covering run.
- **Identity** — a `nodes` key must equal its node's `id`. The two
  spellings address different graphs (the cycle check walks keys,
  `ChildIndex` walks `node.id`), and a mismatch let a self-loop past
  the cycle rejection into a scene build that never terminates.
- **Mutation timing** — `custom_mutations` and `inline_mutations`
  are walked for their animation envelopes, since a map's own
  mutation fires on a click and would otherwise deliver its numbers
  one interaction after the checks. The *mutator payload* itself —
  the `MutatorNode` AST, whose leaves can set a font scale directly
  — is deliberately **not** screened here. It is defended the other
  way, by clamping where the value lands (`GlyphArea`'s scale
  setters and its delta path), because console verbs, macros and IPC
  write the same fields without passing the loader at all. Screening
  the AST as well would be a second wall, not the only one.
- **File size** — `MAX_MAP_BYTES`, checked by `stat` before the read
  commits to the allocation.

Two structural defenses sit behind it. Every walker over `parent_id`
depth is **iterative** — a linear chain is a legal acyclic tree, and
recursing over one exhausted the stack and killed the process from
`fold_hidden_set`, the scene build, the subtree-AABB pass, and the
BVH hit-test. Three more sit outside `baumhard` and are held to the
same rule: `walk_drag_subtree`
(`src/application/document/hit_test.rs`), which died on the first
*drag* over a deep map — after the load and the scene build had both
survived, which is exactly what made it easy to miss;
`arena_utils::clone_subtree`, which no shipping path reaches today
but is `pub`, with the undo stack as its named consumer; and
`maptool`'s legacy id assignment (`convert/ids.rs`), which walks a
*legacy* file's parent chain during `convert --legacy` — a file from
wherever the user got it, so the same depth argument applies to the
migration tool as to the editor. And the
font metric is **clamped where it lands**
(`GlyphArea`'s scale setters), not only at the door, because console
verbs, macros, and IPC write there without passing the loader at
all.

`maptool verify` deliberately does *not* inherit this door: it uses
`loader::parse_for_inspection`, because the files worth asking about
are exactly the ones the editor refuses, and "cannot load" is the
one answer the user already has.

### Threat model

Map and Inline tiers ship today, and **a third-party
`.mindmap.json` is still a privilege event** — a smaller one than it
was, but the capability surface above is what decides that, not the
numeric screening. Treat third-party mindmap files as code, not
data:

- A hostile mindmap can bind any non-destructive Action to a hotkey
  the user is likely to press (Enter, Tab, etc.).
- A hostile mindmap can carry CustomMutation steps that mutate
  document state in surprising ways (the changes are in-memory and
  undoable; the worst case is an annoying user experience, not data
  exfiltration).
- A hostile mindmap CANNOT run console verbs, save the file, delete
  selection, or touch the clipboard — those are all gated above.
- A hostile mindmap can no longer *crash* the editor with a number:
  the cases that did — a zero font size, an inverted clamp, absurd
  geometry, a deep `parent_id` chain, a key that disagreed with its
  node's `id` — are refused at load or bounded where they land. See
  the trust boundary above.

**What is not yet closed**, so nobody reads the section above as a
finished job:

- The resource caps are mostly **per element**, and element count is
  not capped. One border side cannot emit more than 100k glyphs, and a
  map may carry a great many sides; the aggregate per frame is still
  unbounded for everything except connection glyphs.
  A frame budget is the real answer and exists only for that one axis.

  Connection sampling *is* now bounded in aggregate:
  `MAX_TOTAL_PATH_SAMPLES` (500k) is divided equally among the map's
  edges, because the per-path cap alone let a 73 KB file with 200 edges
  ask for 2 000 000 glyph areas — a 2 000 201-node arena and 1 642 MiB
  resident, from a file smaller than any map in this repository. The
  same file now reaches 500 000 samples and 416 MiB, and no document
  can exceed that however many edges it declares.
  `maps/testament.mindmap.json` (16 259 samples over 258 edges) and
  `maps/stress_long_edges.mindmap.json` (46 290 over 124) are
  unaffected — the budget is an order of magnitude above the heavier
  of them.

  **The share is equal per edge, not first-come**, so the outcome does
  not depend on iteration order and no edge renders while a later one
  vanishes. What degrades under a pathological map is glyph *density*
  along every connection at once, which is the graceful form of this
  failure. A map with a hundred thousand edges gets five glyphs each;
  that is thin, and it is the trade against not opening at all.
- **Edge hit-testing re-samples curved edges on every click.**
  `hit_test_edge` walks each Bezier from scratch per canvas click
  rather than reusing the samples the scene build already produced,
  so a map full of long curved edges makes every click expensive.
  The per-path cap bounds one edge; nothing bounds the sum, and the
  fix is a cache or a spatial pre-filter rather than a smaller cap.
- The `OnClick` trigger path consults no `SourceTier`. Today's two
  `DocumentAction` variants are pure in-memory theme writes, so the
  reachable damage is bounded — but the gate the privilege model
  describes lives on the macro dispatcher only, and a new variant
  would widen this silently. `DocumentAction`'s `#[non_exhaustive]`
  is the reminder; an `allows_document_action` sibling to
  `allows_action` is the fix.
- `MAX_FONT_SIZE_PT` combined with the maximum camera zoom can still
  ask the rasterizer for a very large glyph.
- **The browser's `?map=` takes a whole URL, not a path.** The value
  goes to `fetch` verbatim, so an absolute one loads a third-party
  map cross-origin (the attacker's own server decides its CORS). The
  content is screened like any other — the WASM path runs
  `load_from_str` — but the *provenance* step is gone: there is no
  file the user chose to open, only a link they clicked. Restricting
  the parameter to a same-origin path is a product decision, not a
  bug fix, which is why it is named here rather than changed.
- **Map and Inline outrank User in the tier order.** That is
  deliberate — a map should be able to specialize behavior for its
  own content — but it means an opened map shadows the user's own
  macro on an id collision, so a key bound in
  `~/.config/mandala/macros.json` can do what the map chose instead.
  The gates bound *what* that is (no console verbs, nothing
  destructive), and the shadowing is reversible: closing the map
  clears the Map tier and the user's entry re-emerges. Reversing the
  order is a format-wide decision touching the six places the
  SOURCE-OF-TRUTH comment above names.
- A node's `sections` array is capped *after* serde has already
  allocated it, so the cap bounds the model rather than the parse.
  `MAX_MAP_BYTES` is what actually bounds the allocation today.

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
parameterized verb (`border preset=triple`, `color bg=#fafafa`,
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
