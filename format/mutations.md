# Custom Mutations

Custom mutations are named, reusable bundles of operations a mindmap
can attach to nodes or expose through the `mutation` console verb.
They cover everything from "grow every descendant's font by 2pt" to
size-aware layouts like `flower-layout` and `tree-cascade`.

This document is the format reference. For the Rust-side types see
`lib/baumhard/src/mindmap/custom_mutation/`; the loader lives in
`src/application/document/mutations_loader/`.

## Where mutations come from

Four sources contribute to a document's active registry, in ascending
precedence (later writers override earlier ones with the same `id`):

1. **Application bundle** — `assets/mutations/application.json`,
   compiled into the binary via `include_str!`. Lowest precedence so
   users can customize anything shipped by the app.
2. **User file** — `$XDG_CONFIG_HOME/mandala/mutations.json` on
   native (falls back to `$HOME/.config/mandala/mutations.json`);
   `?mutations=<url-encoded-json>` query or
   `localStorage["mandala_mutations"]` on WASM.
3. **Map-level** — the `custom_mutations: [...]` array in the
   `.mindmap.json` file itself.
4. **Inline** — the `inline_mutations: [...]` array on a specific
   `MindNode`.

`mutation help <id>` reports which layer owns the current definition.

## JSON shape

```json
{
  "mutations": [
    {
      "id": "grow-font-2pt",
      "name": "Grow Font 2pt",
      "description": "Increase font size by 2 points on the selected node and all descendants.",
      "contexts": ["map.node"],
      "mutator": {
        "Macro": {
          "channel": 0,
          "mutations": { "Literal": [{ "AreaCommand": { "GrowFont": 2.0 } }] },
          "children": [
            {
              "Instruction": {
                "channel": 0,
                "instruction": "RepeatWhileAlwaysTrue",
                "children": [
                  {
                    "Macro": {
                      "channel": 0,
                      "mutations": { "Literal": [{ "AreaCommand": { "GrowFont": 2.0 } }] }
                    }
                  }
                ]
              }
            }
          ]
        }
      },
      "target_scope": "SelfAndDescendants",
      "behavior": "Persistent"
    }
  ]
}
```

The legacy shape that uses `mutations: [...]` + `target_scope`
instead of a `mutator` AST is still accepted — the backward-compat
deserializer synthesizes the MutatorNode via scope helpers on load.
Save always emits the canonical `mutator` shape. In practice the
legacy shape is often terser for pure-data mutations and is the
preferred authoring form for simple cases.

## Fields

| Field | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | — | Unique key in the registry. `mutation apply <id>` and trigger bindings reference it. |
| `name` | string | — | Human-readable name shown in `mutation list`. |
| `description` | string | `""` | One- or multi-line explanation shown in `mutation list` (first line) and expanded by `mutation help <id>`. |
| `contexts` | string[] | `[]` | Dotted-namespace tags describing where and what the mutation operates on. See `contexts` below. Empty is treated as `["internal"]`. |
| `mutator` | MutatorNode \| null | `null` | The mutation payload. `null` is valid for mutations that only ship `document_actions`. |
| `target_scope` | enum | — | Which nodes the undo path snapshots. See `target_scope` below. |
| `behavior` | `Persistent` \| `Toggle` | `Persistent` | Whether the mutation commits to the model (`Persistent`) or only updates the tree visually and reverses on re-trigger (`Toggle`). |
| `predicate` | Predicate \| null | `null` | Optional filter — elements not matching the predicate are skipped. |
| `document_actions` | DocumentAction[] | `[]` | Canvas-level actions (theme switches, etc.) that fire alongside the tree mutation. |
| `timing` | AnimationTiming \| null | `null` | When present with `duration_ms > 0`, the mutation interpolates over time instead of landing instantly. |

## `contexts`

Tags describing where and on what the mutation is meant to run.
Follows the named-string convention from `enums.md`: unknown tags
are preserved on round-trip but don't match any well-known
predicate.

Well-known tags (defined in
`lib/baumhard/src/mindmap/custom_mutation/contexts.rs`):

- `internal` — an implementation-detail mutation registered by the
  host application for its own use. Not listed by `mutation list`
  and refused by `mutation apply`. An empty `contexts` array is
  equivalent to `["internal"]`.
- `map` — operates on a mindmap. Root of the `map.*` sub-namespace.
- `map.node` — touches the content of a single node (text, style,
  color, regions).
- `map.tree` — touches tree structure / layout descending from a
  node (positions, children arrangement).

Plugins reserve the `plugin.<name>.<kind>` namespace. `mutation list`
filters to entries whose contexts include anything starting with
`map`; `mutation list --all` drops that filter.

## `target_scope`

Scopes the **undo snapshot and model-sync** windows, **not** the
mutator payload's reach. The mutator AST is free to walk wherever its
control-flow dictates; `target_scope` tells the framework which model
nodes to clone into the `UndoAction::CustomMutation` snapshot before
the mutation runs and (for non-handler mutations) which nodes to sync
from the tree back to the model afterward. A mutation author's job is
to keep the declared scope a superset of the nodes the mutator
actually touches — if the mutator writes outside this scope, those
writes won't be reverted by `Ctrl+Z` and won't reach the saved model.

| Value | Nodes snapshotted / synced |
|---|---|
| `SelfOnly` | The anchor node. |
| `Children` | Direct children of the anchor. |
| `Descendants` | All descendants recursively (not the anchor). |
| `SelfAndDescendants` | Anchor + all descendants. |
| `Parent` | The anchor's parent node. |
| `Siblings` | The anchor's siblings (excluding itself). |

For scope-helper-generated MutatorNodes (via
`baumhard::mindmap::custom_mutation::scope::*`) the helper name
matches the `target_scope` value — `scope::self_and_descendants(...)`
pairs with `SelfAndDescendants`, etc. For hand-authored MutatorNodes,
pick the smallest scope that covers every node the AST will touch.

## `mutator = null` (document-actions-only)

A mutation whose payload is only canvas-level work (a theme switch,
an ad-hoc `theme_variables` override) sets `mutator: null` (or omits
the field entirely) and carries its work in `document_actions`. The
framework skips the tree-walk path and runs `apply_document_actions`
alone; the `UndoAction::CanvasSnapshot` captures the pre-action
canvas state for undo. Example:

```json
{
  "id": "switch-dark",
  "name": "Switch to dark theme",
  "description": "Copy the 'dark' theme variant into live variables.",
  "contexts": ["map.node"],
  "target_scope": "SelfOnly",
  "document_actions": [{ "SetThemeVariant": "dark" }]
}
```

`target_scope` is still required on the wire (it's a non-`Option`
field) but is effectively unused for document-actions-only
mutations. `SelfOnly` is the conventional value.

## `mutator` — the MutatorNode AST

The mutator AST mirrors Baumhard's `GfxMutator` variants plus a
`Repeat` wrapper for "N children at consecutive channels". See
`lib/baumhard/src/mutator_builder/ast.rs` for the type definitions.
Four top-level variants:

- **`Void`** — structural grouping, no mutation. Walks children.
- **`Single { channel, mutation }`** — one mutation at one channel.
- **`Macro { channel, mutations, children }`** — a flat list of
  mutations applied to the matched target. `mutations` is
  `{ "Literal": [...] }` for baked-in values or
  `{ "Runtime": "<label>" }` for runtime-supplied values. `children`
  default to empty; used by the `SelfAndDescendants` scope topology
  (a Macro on the anchor whose child is an `Instruction` walking
  descendants).
- **`Instruction { channel, instruction, children }`** — control
  flow wrapping inner children. Instructions:
  - `RepeatWhileAlwaysTrue` — apply children to every descendant.
  - `RepeatWhile(<Predicate>)` — apply children to every descendant
    for which the predicate holds, short-circuit once it fails.
  - `RotateWhile(<f32>, <Predicate>)` — rotation stub (reserved).
  - `SpatialDescend(<OrderedVec2>)` — descend by AABB containment to
    the deepest node that holds the point, deliver the instruction's
    attached mutation.
  - `MapChildren` — **zip-by-sibling-position**. Pairs the
    mutator's direct children with the target's direct children
    by index, **bypassing channel alignment**. See "MapChildren"
    below.
- **`Repeat { section, channel_base, count, skip_indices, template }`**
  — expands at build time into N children at consecutive channels,
  each derived from `template`. `count` is `{ "Literal": N }` or
  `{ "Runtime": "<label>" }`. Used by widgets (the picker's hue
  ring) and any runtime-shaped section.

### Channel alignment vs. `MapChildren`

By default, the walker pairs a mutator's direct children with a
target's direct children **by matching the `channel` field on
each element** — see `format/channels.md`. This is broadcast
semantics: one mutator on channel N hits every target child that
happens to share channel N. It's the right default for groups.

`Instruction::MapChildren` is the opt-in alternative: it pairs
strictly by **sibling position** (zip), ignoring channels entirely.
This is the shape size-aware layouts want — the `i`-th target child
gets the `i`-th mutator child, regardless of how channels are
assigned. A typical declarative layout pairs `MapChildren` with a
`Repeat` expansion fed by a `SectionContext`:

```json
{
  "Instruction": {
    "channel": 0,
    "instruction": "MapChildren",
    "children": [{
      "Repeat": {
        "section": "children",
        "channel_base": 0,
        "count": { "Runtime": "children" },
        "template": {
          "Single": {
            "channel": "SectionIndex",
            "mutation": { "AreaDelta": ["position"] }
          }
        }
      }
    }]
  }
}
```

At apply time the registered `SectionContext` supplies
`count("children") = N` and per-index
`field("children", i, &CellField::position)`; the walker expands the
`Repeat` into N `Single` mutators and `MapChildren` zips them against
the N target children.

### Runtime holes

A MutatorNode may embed runtime variants (`Runtime`, `AreaDelta`
with `CellField`s fed from the area lookup). At apply time the
walker consults a `SectionContext` registered by the host
application for the mutation's `id`. Pure-data mutations (no runtime
holes) use a no-op context.

### Imperative handlers vs. declarative mutators

Two apply paths coexist:

- **Declarative (`mutator: Some(MutatorNode)`)** — the walker
  compiles the AST to a `MutatorTree<GfxMutator>` and walks it over
  the Baumhard tree. The framework syncs mutated nodes back from the
  tree to the model for undo. Runtime holes are resolved via a
  `SectionContext` keyed on the mutation `id`. Preferred for
  mutations expressible in the AST (pure-data field changes,
  MapChildren-shaped per-index layouts, predicate-gated recursion).
- **Imperative (`DynamicMutationHandler`)** — a Rust function
  pointer registered on the document under the mutation `id`. When
  present, the dispatcher calls the handler directly and it mutates
  the `MindMap` model in place, bypassing the tree walk. Chosen for
  mutations too structural for the AST (arbitrary BFS layouts,
  anything that needs per-node custom computation spanning multiple
  passes). The mutator field in the JSON is conventionally empty
  (`"mutations": []` in the legacy shape, or `mutator: null`).

Decision rule: reach for the declarative path first. Move to a
handler when the AST gets contorted — typically when you find
yourself wanting per-target state that `SectionContext` can't
cleanly express, or multiple tree passes. `flower_layout.rs` and
`tree_cascade.rs` under `src/application/document/mutations/` are
the canonical handler examples; both are registered by
`register_builtin_handlers` at startup.

## Authoring a user file

Drop a JSON file at `~/.config/mandala/mutations.json` with the
envelope shape above. The file is read at startup; `log::info!`
reports the count on success, `log::warn!` on parse failure.
Malformed files don't crash the app — the layer is skipped and the
app bundle below continues to load.

A user mutation whose `id` matches an app-bundled mutation overrides
the bundle — useful for tweaking constants without rebuilding. A
map-level mutation in turn overrides the user's, and an inline
mutation on a specific node overrides the map's.

## Related

- `format/schema.md` — the full `.mindmap.json` shape including the
  `custom_mutations` and `trigger_bindings` arrays on maps and nodes.
- `format/channels.md` — the `channel` field on MindNodes and what
  it means for mutator targeting.
- `format/enums.md` — the named-string convention used for
  `contexts`, `target_scope`, `behavior`.
- `CODE_CONVENTIONS.md §1` — why the mutation framework lives in
  Baumhard and why its seams are preserved.
