# Custom Mutations

Custom mutations are named, reusable bundles of operations a mindmap
can attach to nodes or expose through the `mutation` console verb.
They cover everything from "grow every descendant's font by 2pt" to
size-aware layouts like `flower-layout` and `tree-cascade`.

This document is the format reference. For the Rust-side types see
`lib/baumhard/src/mindmap/custom_mutation/`; the loader lives in
`src/application/document/mutations_loader/`.

## Where mutations come from

Four sources contribute to a document's active registry, in
ascending precedence (later writers override earlier ones with the
same `id`):

<!-- SOURCE-OF-TRUTH: the precedence order below is encoded once in
     code, as the SourceTier enum variant order in
     src/application/source_tier.rs, and pinned by that module's
     tests. The macro registry shares the same enum. When the order
     or set of sources changes, update this list and
     build_mutation_registry_with_app_and_user in the same commit. -->

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

`mutation help <id>` and `mutation inspect <id>` both report which
layer won the registry slot for that id.

Override-safety note: if a user file redeclares the id of a
bundled mutation that has a registered Rust handler (e.g.
`flower-layout`, `tree-cascade`), the dispatcher **honors the
user's declarative mutator** rather than silently running the
bundled handler's algorithm against the user's scope. See
`MindMapDocument::will_dispatch_to_handler` for the guard.

## JSON shape — start here

Most mutations are pure data: a flat `Vec<Mutation>` applied over a
declared scope. Use this shape:

```json
{
  "mutations": [
    {
      "id": "grow-font-2pt",
      "name": "Grow Font 2pt",
      "description": "Increase font size by 2 points on the selected node and all descendants.",
      "contexts": ["map.node"],
      "mutations": [{ "AreaCommand": { "GrowFont": 2.0 } }],
      "target_scope": "SelfAndDescendants"
    }
  ]
}
```

Drop that at `~/.config/mandala/mutations.json`, restart the app,
and `/` → `mutation list` will show it. `mutation apply grow-font-2pt`
on a node's selection grows the fonts. Three lines of change to
author a new mutation.

Fields the shape above uses:

- `id` — unique key, typed at the console prompt.
- `name` — human-readable name in `mutation list`.
- `description` — first line shown in `mutation list`, expanded by
  `mutation help <id>`.
- `contexts` — dotted tags describing what the mutation touches
  (`map.node`, `map.tree`, `internal`, …). See [contexts](#contexts)
  below.
- `mutations` — a flat `Vec<Mutation>` applied to every node
  covered by `target_scope`. See the [Mutation vocabulary](#mutation-vocabulary)
  section for the variants.
- `predicate` — optional `Predicate` filter gate; defaults to none.
  See [predicate](#predicate--top-level-filter-gate) below.
- `target_scope` — one of `SelfOnly` / `Children` / `Descendants` /
  `SelfAndDescendants` / `Parent` / `Siblings` / `SectionsOnly`.
  Governs both what nodes the mutations apply to *and* the
  undo-snapshot window.

When the legacy shape isn't enough — a mutation that needs
per-child positioning, runtime-computed values, or the
`MapChildren` walker primitive — use the richer [MutatorNode
AST](#mutator--the-mutatornode-ast) form instead of or in addition
to `mutations`. The backward-compat deserializer accepts both
shapes on load; save always emits the canonical `mutator` form.

### Mutation vocabulary

`Mutation` is an enum from `baumhard::gfx_structs::mutator`. The
JSON tag is the variant name; the payload is the variant's inner
value. The most common variants for authoring:

- `{"AreaCommand": { "GrowFont": 2.0 }}` — grow font size by 2pt.
- `{"AreaCommand": { "ShrinkFont": 2.0 }}` — inverse.
- `{"AreaCommand": { "NudgeRight": 50.0 }}` — shift x by 50px.
  Sibling variants: `NudgeLeft`, `NudgeUp`, `NudgeDown`.
- `{"AreaCommand": { "SetFontSize": 18.0 }}` — absolute font size.
- `{"AreaCommand": { "MoveTo": [100.0, 200.0] }}` — absolute x,y.
- `{"AreaCommand": { "Rotate": { "pivot": [0.0, 0.0], "degrees": 90.0 } }}`
  — rotate the area's position clockwise around `pivot`. Degrees,
  not radians; clockwise in screen space, where `+y` points down.
  Mirrors its model-side twin `{"ModelCommand": { "Rotate": … }}`.

  **`pivot` is a two-element `[x, y]` array, not an
  `{"x": …, "y": …}` object.** The *variant* is struct-shaped (named
  members `pivot` and `degrees`), but the `Vec2` inside it is
  serialized by `glam` as a sequence, and its deserializer accepts
  nothing else. The object form fails with
  `invalid type: map, expected a sequence of 2 f32 values`, and
  because `custom_mutations` is a required-shape field, that error
  takes the **whole `.mindmap.json` down** — or, in
  `~/.config/mandala/mutations.json`, makes `load_user` warn and
  silently drop the entire user mutation file. The same applies to
  every future `Vec2` payload on either command enum.
  `do_area_rotate_command_json_wire_shape` **reads the example on the
  line above out of this file** and parses it, so editing it back to
  the object form fails the suite rather than shipping a doc that
  breaks every document that copies it.

The full vocabulary lives in
`lib/baumhard/src/gfx_structs/area_mutators.rs` under
`GlyphAreaCommand`. Same enum-variant-as-JSON-tag convention.

> **Added (pre-V1, per `CODE_CONVENTIONS.md` §10).** `Rotate` is
> new on the area side; the model side already had it. The addition
> is purely additive — no existing key changed meaning, so no
> in-repo fixture, bundled asset, or `maptool` path needed
> migrating. Previously `GlyphArea::rotate` existed in Rust but no
> command could reach it, which is why its missing translate-back
> (it rotated the pivot-relative vector and never added the pivot
> back, teleporting the area toward the origin) went unnoticed.

### Delta mutations and their field-map keys

`{"AreaDelta": …}` and `{"ModelDelta": …}` carry a *set* of field
payloads rather than one named command. On the wire the payload is
an object with a single `fields` member, and **the keys of that map
are field-type tag names** — one per touched field, plus a sibling
`Operation` entry naming the arithmetic that governs all of them:

```json
{
  "AreaDelta": {
    "fields": {
      "Position": { "Position": { "x": 100.0, "y": 200.0 } },
      "Operation": { "Operation": "Assign" }
    }
  }
}
```

The valid keys are exactly the variants of `GlyphAreaField`
(`Text`, `Scale`, `LineHeight`, `Position`, `Bounds`,
`ColorFontRegions`, `Outline`, `Shape`, `ZoomVisibility`,
`Operation`) and of `GlyphModelField` (`GlyphMatrix`, `GlyphLine`,
`GlyphLines`, `Layer`, `Position`, `Operation`). Both tag sets are
*derived* from the field enums, so the key list can never drift from
the fields that actually exist.

A `GlyphLine` payload (used directly by `GlyphLine`/`GlyphLines`
and nested inside `GlyphMatrix`) carries a required
`ignore_initial_space` boolean alongside its `line` array of runs.
When it is `true` the rhs's leading whitespace is *transparent*:
the all-whitespace runs in front are skipped, and the first run
carrying content paints at its own grapheme offset — its indent
counted into the offset, not written over the target. An rhs that
is entirely whitespace therefore paints nothing at all.

Every run after that first one paints at **its own** grapheme
offset within the rhs, in order. Offsets are columns, not run
ordinals: the rhs's run boundaries need not line up with the
target's, and after the first paint they generally do not. Two
rhs payloads that spell the same text with different run
boundaries produce the same result; only the column each run
starts at matters. Runs that reach past the end of the target
extend it, padding any gap with whitespace.

> **Behavior change (pre-V1, per `CODE_CONVENTIONS.md` §10).** All
> the statements above are new. The path previously mixed byte,
> `char`, and grapheme offsets, so an indent containing a
> multi-byte space (U+3000) panicked outright and a multi-char
> cluster (CRLF) painted one column off; runs after the first were
> placed by run ordinal against the target rather than by their own
> column, which reordered and mislaid them; an rhs with more runs
> than the target also panicked; and an all-whitespace rhs blanked the
> target instead of showing through. No in-repo fixture or bundled
> asset set the flag, so there was nothing to migrate — but a
> hand-authored file that relied on the old blanking behavior needs
> `ignore_initial_space: false` to keep it.

> **Breaking change (pre-V1, per `CODE_CONVENTIONS.md` §10).** The
> area-side operation key was previously spelled `ApplyOperation`;
> it is now `Operation`, matching the field variant it tags and the
> model-side key of the same meaning. The write-only keys `Flags`
> (both sides) and the `SetFlag` command tag have been removed —
> they named no field or command and were never applicable. A
> hand-authored `.mindmap.json`, `~/.config/mandala/mutations.json`,
> or `assets/mutations/*.json` still using `ApplyOperation` or
> `Flags` as a map key will fail to deserialize with
> `unknown variant`, and because mutations are parsed as part of the
> enclosing document, **the whole file fails to load** — not just
> the offending mutation. Rename the key to `Operation`; drop any
> `Flags` entry. No alias or migration shim is provided (§10:
> rename rather than alias). No in-repo fixture, bundled asset, or
> `maptool` path used these keys, so there was nothing to migrate in
> the same commit.

### Document-actions-only mutation

A mutation can carry only canvas-level work (e.g. a theme switch)
with no tree effect at all. Omit `mutator` and `mutations`, keep
`target_scope` as a formal placeholder:

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

`UndoAction::CanvasSnapshot` captures the pre-action canvas state
so `Ctrl+Z` reverses it.

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
| `Parent` | The anchor's parent node. Empty on a root — the mutation is a no-op. |
| `Siblings` | The anchor's siblings (excluding itself). **Empty on a root**: "sibling" means "shares my parent", and a root shares none, so the other roots of a multi-root map are *not* siblings. A `Siblings` mutation on a root snapshots nothing, mutates nothing, and pushes no undo entry. |
| `SectionsOnly` | Every section of the anchor — bypasses the chrome-only container fan-out so text / font / region mutations land on the section-areas only. The anchor `MindNode` is still the snapshot window (whole-node clone covers per-section state). Use this when a mutation must avoid colliding with a sibling mind-node sharing the section's channel. |

For scope-helper-generated MutatorNodes (via
`baumhard::mindmap::custom_mutation::scope::*`) the helper name
matches the `target_scope` value — `scope::self_and_descendants(...)`
pairs with `SelfAndDescendants`, etc. For hand-authored MutatorNodes,
pick the smallest scope that covers every node the AST will touch.

### Which scopes admit a tree-walking mutator

The application resolves a scope to a target set and then **anchors
the mutator at each target in turn**. So the pairing has complete
undo coverage only when the target set is *closed* under whatever
the mutator walks: everything a mutator anchored at a target can
reach must itself be a target, or the undo snapshot ends up narrower
than the write set.

Only `Descendants` and `SelfAndDescendants` are closed — a
descendant's children are still descendants. Every other scope,
including `Children` and `Siblings`, requires a mutator that touches
**only its anchor**: anchoring a `MapChildren`-reach mutator at each
child reaches the *grandchildren*, and at each sibling reaches that
sibling's children, neither of which the snapshot captured.

Closure is a statement about **undo coverage, and nothing else**. It
does not say the mutation applies once per node. Per-target
anchoring means a mutator with a reach wider than `SelfOnly` runs
once for *every* target that reaches a given node, so under a
`SelfAndDescendants` scope a `Descendants`-reach mutator is anchored
at each node of the subtree and a node at depth *k* below the anchor
is written *k + 1* times. The snapshot still covers all of it —
closure holds, `covers_reach` is right to approve — but a
non-idempotent payload (a relative nudge, say) compounds. Today's
flat-apply path collapses the AST to one list and applies it once
per target, so nothing compounds yet; the announced walker path is
where this becomes real.

`baumhard::mindmap::custom_mutation::mutator_reach` computes the
widest set an AST can reach and `TargetScope::covers_reach` checks
the pairing; a mismatch is a `warn!` at apply time, not a rejection —
the mutation still runs, but its undo coverage is incomplete. The
`mutation inspect <id>` console verb prints the computed reach.

One caveat on "still runs", because it bites precisely the pairings
this gate rejects. The pairings that trip it — a `Children` or
`Siblings` scope against a `MapChildren`-reach mutator, say — are by
construction *not* flat-extractable, and the flat-apply path is the
only path wired today. So such a mutation collects its second warning
from `apply_to_tree` and is **skipped**: the `covers_reach` warning
is advisory, but the non-flat decline behind it is not. "Still runs"
is accurate for a flat-extractable AST whose declared scope is merely
too narrow, and for those only.

## `predicate` — top-level filter gate

Optional `Predicate` that narrows the candidate elements
*before* the mutator's `Macro` payload lands on each target.
When present, every element in the `target_scope`-collected
fan-out (the chrome-only container plus each section-area) is
tested against `predicate.test(element)`; mutations only land on
elements that match.

The shape mirrors the predicates used inside
`Instruction::RepeatWhile` — same struct, same field language —
so authors don't learn two filtering grammars. Per-grapheme /
per-region targeting still happens inside the mutator AST; the
top-level `predicate` is the *element-level* filter ("does this
GlyphArea / GlyphModel even count?").

The predicate field language exposes `GfxElementField::Flag(Flag)`
for the most common authoring shape — "match elements with this
flag set / clear". The `Flag` variants are:

| Variant | Set on |
|---|---|
| `Focused` | Element holds keyboard / interaction focus. |
| `Mutable` | Element accepts user-driven mutation. |
| `Anchored(AnchorBox)` | Element is layout-pinned. |
| `MutationEvents` | Element generates events on mutation. |
| `SectionRoot` | Element is part of a section subtree (section-area or section-model). |

Common idioms:

- `Predicate { fields: [(Flag(SectionRoot), Equals(false))], always_match: false }`
  — "land on sections only" (filters the container out).
  Same end-state as `target_scope: SectionsOnly` for a
  `SelfOnly`-style mutator; the structural scope is preferred
  when authoring intent is "sections, never the chrome".
- `Predicate { fields: [(Flag(SectionRoot), Equals(true))], … }`
  — the inverse: "land on the container only, skip every
  section." Useful for chrome-only mutations (background,
  border) that shouldn't touch the section glyphs.

`predicate` is `null` (or absent) by default — every candidate
element passes through unfiltered. The field round-trips through
serde with `skip_serializing_if = "Option::is_none"`, so
mutations that don't filter stay byte-identical on disk.

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
    Note the predicate genuinely filters: a `RepeatWhile` whose
    predicate is the bare `{ "fields": [] }` shape (no fields,
    `always_match` absent/false) matches **nothing**. Today's
    flat-apply path has no predicate evaluator, so it declines any
    `RepeatWhile` that isn't `always_match` and warns instead of
    applying — the alternative would be landing the payload on the
    whole scope set, the inverse of what the AST says.
    Extraction is **all-or-nothing**: one such `RepeatWhile`
    *anywhere* in the AST declines the whole mutator, nested just as
    much as at the root. Honoring a root while dropping a nested
    branch would be the worse failure of the two — the root's payload
    would blanket every target, the nested payload would land
    nowhere, and nothing would warn.

    The same argument decides shapes with **no** unevaluatable node
    in them. `Macro{[L1], children: [Macro{[L2]}]}` is extractable
    top to bottom and still declines, because one flat list cannot be
    both `L1` and `L2` — picking `L1` lands `L2` nowhere, which is the
    identical failure by a different road. So the rule is: every
    payload anywhere in the AST must **agree with** the one
    extracted. `scope::self_and_descendants` satisfies it exactly
    (its root and nested `Macro` carry two clones of the same list);
    nesting *differing* payloads is declined, not merged.
    Concatenating them instead would turn that helper into a
    double-apply.

    "Agree" is *would write the same thing*, which is deliberately
    not `==` on the payload: the numeric fields are `f32`, so `==`
    is not reflexive and a `NaN` would make the duplicated payload
    of `scope::self_and_descendants` disagree with **itself** and
    decline, while the same payload under `scope::self_only` — with
    nothing to compare — applied. Two payloads agree when they are
    `==` **or** structurally identical, so `NaN` agrees with itself
    and `0.0` still agrees with `-0.0`. `NaN` does not agree with
    `inf`.

    Two corollaries of "every payload must agree":

    - An `Instruction` wrapper carrying its own `mutation` — a
      `Runtime` hole or an `AreaDelta` per-cell template — is
      declined. Both are payloads the flat path provably cannot
      evaluate. The scope helpers all set `"mutation": "None"`.
    - A `Void` **on channel 0** is transparent, since it carries no
      mutation of its own: an empty one cannot lose anything and does
      not decline, while one wrapping a disagreeing payload still
      surfaces the disagreement. Off channel 0 it declines — a
      `Void`'s channel is branch routing (the walker aligns its
      children only against the target child on that channel), and
      the flat path produces one list applied to whole elements with
      nothing to route on. All the scope helpers build on channel 0.
      `Single` declines in every form, including
      `"mutation": "None"`: it is a leaf whose `channel` selects the
      target it writes, so admitting it would widen the flat path
      into precisely the routing it cannot honor.
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

## Firing mutations from interaction

The `mutation` console verb is one way to fire a mutation. The
other is a trigger binding: a mutation id attached to a node's
`trigger_bindings` array, fired when the user clicks, hovers, or
presses a bound key on that node. The binding JSON shape lives on
the node, not on the mutation:

```json
{
  "id": "0",
  "parent_id": null,
  "text": "Root",
  "trigger_bindings": [
    { "trigger": "OnClick", "mutation_id": "switch-dark" },
    { "trigger": "OnHover", "mutation_id": "highlight-red",
      "contexts": ["Desktop"] }
  ]
}
```

Valid triggers: `OnClick`, `OnHover`, `{"OnKey": "<key>"}`,
`{"OnLink": "<href>"}`. The optional `contexts` field limits the
binding to particular runtime platforms (`Desktop` / `Web` /
`Touch`); omit it to fire on all platforms.

Trigger bindings respect `MutationBehavior::Toggle` semantics —
click the same node twice on a Toggle-flavoured mutation to
reverse it. See `format/schema.md` for the full node field
reference.

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
- `format/zoom-bounds.md` — `GlyphAreaField::ZoomVisibility` mutator
  target for authoring zoom-triggered LOD transitions on any
  renderable.
- `CODE_CONVENTIONS.md §1` and `§7` — why the mutation framework lives
  in Baumhard and why its seams are preserved.
