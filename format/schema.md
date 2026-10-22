# Schema Reference

Complete field reference for every type in `.mindmap.json`.

## Unknown keys are kept

Every object in a `.mindmap.json` is **open**. A key that no field
claims is not a load error and it is not quietly dropped: the loader
warns once, naming where it found it, and writes the key back
unchanged the next time the map is saved.

The reason is version skew in both directions. A map authored by a
newer build carries keys an older build has never heard of. Refusing
the document would leave the reader with nothing at all — the map they
can mostly render, they cannot see. Loading it and forgetting the keys
is worse: Mandala is an editor, it writes the whole model back, and a
key dropped at load is a key **deleted from the file at the next
save**. For a hand-authored map that file was the only copy. So the
loader keeps what it does not understand, and an older build can open,
edit and resave a newer map without destroying the newer features.

This map loads:

```json
{
  "version": "1.0",
  "name": "typo",
  "canvas": { "background_color": "#000000" },
  "nodes": {
    "0": {
      "id": "0",
      "parent_id": null,
      "position": { "x": 0.0, "y": 0.0 },
      "size": { "width": 200.0, "height": 60.0 },
      "sections": [ { "text": "Hello" } ],
      "style": {
        "background_color": "#141414",
        "frame_color": "#30b082",
        "text_color": "#ffffff",
        "shape": "rectangle",
        "corner_radius_percent": 10.0,
        "frame_thickness": 4.0,
        "show_frame": true,
        "show_shadow": false
      },
      "layout": { "type": "map", "direction": "auto", "spacing": 50.0 },
      "folded": false,
      "notes": "",
      "color_schema": null,
      "channel": 0,
      "min_zoom_to_rendr": 2.0
    }
  },
  "edges": []
}
```

and says so:

```
loader: node "0": unrecognized key `min_zoom_to_rendr` — this build
has no field for it, so it is kept as written and saved back with the
value it was authored with. Check the spelling if you meant an
existing key; see format/schema.md.
```

The node renders, `min_zoom_to_render` is unset because nothing set
it, and `min_zoom_to_rendr` is still in the file after the next save.
That last part is the whole policy: a key you keep is a key you can
still fix. Fix the spelling and the warning stops:

```json
{
  "version": "1.0",
  "name": "typo-fixed",
  "canvas": { "background_color": "#000000" },
  "nodes": {
    "0": {
      "id": "0",
      "parent_id": null,
      "position": { "x": 0.0, "y": 0.0 },
      "size": { "width": 200.0, "height": 60.0 },
      "sections": [ { "text": "Hello" } ],
      "style": {
        "background_color": "#141414",
        "frame_color": "#30b082",
        "text_color": "#ffffff",
        "shape": "rectangle",
        "corner_radius_percent": 10.0,
        "frame_thickness": 4.0,
        "show_frame": true,
        "show_shadow": false
      },
      "layout": { "type": "map", "direction": "auto", "spacing": 50.0 },
      "folded": false,
      "notes": "",
      "color_schema": null,
      "channel": 0,
      "min_zoom_to_render": 2.0
    }
  },
  "edges": []
}
```

A warning is easy to miss, so `maptool verify` reports the same keys
as violations and exits non-zero — that is where a typo is meant to be
caught, alongside every other thing that is spelled fine and means
nothing. See [`validation.md`](./validation.md).

The warning names the **part of the document** that carries the key —
`node "1.2"`, `edge[3]`, `palette "coral"`, `canvas`,
`custom_mutations[0]`, or `map` for a key at the top level — followed
by the field path inside that part (`style.shpe`,
`sections[0].txet`). Not a byte offset: a map runs to thousands of
lines and the key is the thing you can search for.

**Three shapes are still refused**, and all three are pre-refactor
spellings rather than keys from the future: a top-level `portals`
array, per-node `text` / `text_runs`, and a `sections` that is not an
array. The current model means something else by those names, so
carrying them forward would carry a contradiction forward. Each is
answered with the `maptool convert` verb that migrates it; see
[`migration.md`](./migration.md).

**A construct built on a name this build does not know is skipped, not
fatal.** An unrecognized *key* is inert — nothing reads it, so ignoring
it changes nothing. An unrecognized **variant** is the opposite: it is
the instruction. `{"mutator": {"Glow": …}}` from a newer build used to
make the whole document unloadable, which is the empty window this
policy exists to avoid. The load now lifts the construct out, opens the
rest of the map, warns saying that nothing it describes will run, and
writes it back at the index it was authored at, with every value it
carried intact.

The unit that is skipped is a whole custom mutation (`custom_mutations[i]`
or a node's `inline_mutations[i]`) or a whole trigger binding (a node's
or a section's) — never the part inside it that failed. Dropping one
`Mutation` out of a macro would leave a custom mutation that still
appears in `mutation list`, still fires, and silently does less than it
says; dropping the entry is visible as absence. Nothing else is
skippable: a node, an edge, the canvas or a palette this build cannot
read still fails the load, because a map missing part of itself with no
sign of which part is worse than no map.

`maptool verify` still reports every skipped construct as a violation
and exits non-zero, naming the variant. Loading a map and validating it
are different questions, and `{"mutator": {"Glwo": …}}` is a typo that
still needs a moment at which somebody finds out.

**What forward compatibility covers, exactly.** Three things, and it is
worth being precise because the boundary is not obvious:

- **Unknown object keys**, anywhere in the document — kept, warned,
  written back.
- **Unknown named-enum strings** (`"shape": "hexagram"`,
  `"line_style": "dot-dash"`) — these are `String` fields in the model
  with a documented fallback, so they load, render as the default, and
  survive the save unchanged. See [`enums.md`](./enums.md).
- **Unknown externally tagged enum variants** inside a custom mutation
  or a trigger binding (`MutatorNode`, `Mutation`, `MutationSrc`,
  `MutationListSrc`, `DocumentAction`, `Trigger`, `InstructionSpec`,
  `ChannelSrc`, `CountSrc`) — the construct is skipped whole and
  preserved, as above.

It does **not** cover a structural change to a part that cannot be
skipped: a node, an edge, the canvas or a palette whose shape this build
cannot read still fails the load.

**What "preserved" guarantees, precisely: the value, not the bytes.**
Every preserved key comes back with exactly the value it was authored
with — `1.0` stays `1.0`, an integer past 2^53 keeps every digit,
`0.30000000000000004` keeps all seventeen, an emoji stays a literal
emoji rather than a surrogate escape, and a `\n` inside a string stays
an escape. What is *not* promised is the spelling: a save renders the
whole document through one writer, so an author's `1e2` comes back as
`100.0`, `1.5E-3` as `0.0015`, and members are written in sorted order
wherever they were authored. That is the same normalization every
key this build does understand gets — preserved keys are not treated
specially, which is the point. Nothing that carries information is
lost, and a `diff` against the authored file will still show
un-edited lines moving.

**Where a preserved key can still be lost.** A key is written back at
the route it was read from. Above an array — a node by its id, a palette
by its name — that route is stable across any edit. Below one, it is
positional. These are the arrays a captured key's route can cross:

```
GlyphLines
Literal
children
control_points
custom_mutations
edges
fields
groups
inline_mutations
line
matrix
mutations
regions
sections
text_runs
trigger_bindings
```

The two capitalized names are not fields. An externally tagged enum
writes its payload as the single member of a wrapper object, so a
variant whose payload is an array is an array published under the
*variant's* name: `{"Literal": [ … ]}` inside a macro's mutation list,
`{"GlyphLines": [ … ]}` inside a model-field delta.

That list is **derived from the model, not kept by hand**:
`unknown_keys::tests::test_the_published_positional_arrays_are_the_ones_the_model_has`
walks the load graph and fails in both directions, because the
hand-written version of it had already drifted past `control_points`
and a palette's `groups`. `macros` and `inline_macros` are arrays and
are deliberately *not* on it — their elements are opaque JSON, so
nothing inside one is ever reported as unrecognized and no route
crosses their indexes.

**Derived is not the same as right, and this list has been wrong
while saying it was derived.** The walk behind it read the model's
source text, and three of the sixteen names above were missing from
it until [#122](https://github.com/Svendsys/mandala/issues/122):
`regions` is a `BTreeSet`, which is an array on disk but does not
spell the token `Vec`, and `GlyphLines` and `Literal` are the variant
payloads described above, which the walk skipped for having no field
name. The document matched the walk exactly, which is why nothing
noticed. The list is now cross-checked against a second derivation
that shares nothing with the first — `util::serde_probe` asks the
*generated* `Deserialize` impls what they expect, so the compiler's
own expansion of the model has to agree before this block can be
called correct. Fixed-arity arrays — a tuple, a tuple variant's
payload slots — are still absent on purpose: a route does cross their
indexes, but no edit can move one without changing the format.

Position alone is not trusted there: the load records what each array
looked like, so the save re-finds the element the key was attached to
after a deletion or a reorder, and keeps it through an edit to that
element. Three cases still lose the key, and all three say so at
`warn!`:

- the route is gone entirely — the node, edge or section it hung off was
  deleted, and the key goes with it;
- the element can no longer be identified — it was edited *and* its
  siblings changed, so neither its content nor its position is evidence.
  It is dropped rather than written onto whatever now sits at the index:
  wrong data reads as authored, missing data does not;
- this build has since grown a field of that name at that place, and the
  value the model writes wins.

A **zero-edit** load → save loses nothing, in any position — including a
key nested inside a container the saver omits because it holds its own
default (a section's `offset`), and a key below a `#[serde(from = "…")]`
proxy where the shape that is read (`mutations`) is not the shape that is
written (`mutator`). In that last case the legacy `mutations` list is
written back alongside the upgraded `mutator` so the key inside it has
somewhere to live; `mutator` takes precedence on reload, so the model is
unaffected.

The converse is not a loss: a key written out at its own default value
(`"color_schema": null`, `"text_runs": []`) may be *omitted* when the
map is saved. Nothing that carries information disappears — reload the
saved file and the model is the same one.

**What this does not cover.** Openness is about *keys*, not
*meanings*. An edge whose `to_id` names no node, a `color_schema`
pointing at a palette that isn't there, a section that hangs outside
its node — all of those are spelled correctly and all of them load.
They are `maptool verify`'s job; see [`validation.md`](./validation.md).

Values inside `macros` and a node's `inline_macros` are the deliberate
exception: baumhard stores them as opaque JSON (the typed `Macro` lives
in the application crate), so their interiors are carried through
untouched and never reported as unrecognized. See
[`macros.md`](./macros.md).

## Top-level object

```json
{
  "version": "1.0",
  "name": "map-name",
  "canvas": { ... },
  "palettes": { ... },
  "nodes": { ... },
  "edges": [ ... ],
  "custom_mutations": [ ... ],
  "macros": [ ... ]
}
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `version` | string | yes | Format version string |
| `name` | string | yes | Human name (usually derived from filename) |
| `canvas` | object | yes | Canvas rendering context |
| `palettes` | object | no | Named color palettes keyed by name |
| `nodes` | object | yes | Node map keyed by ID |
| `edges` | array | yes | Ordered edge records (can be empty). Portals are edges with `display_mode = "portal"` — no separate top-level collection. |
| `custom_mutations` | array | no | Map-level reusable mutations |
| `macros` | array | no | Map-level macro definitions (`Map` privilege tier — cannot run `ConsoleLine` or destructive Actions). Stored as opaque JSON in baumhard; the application crate parses each entry. Full reference: [`macros.md`](./macros.md). |

## Canvas

```json
{
  "background_color": "#000000",
  "default_border": null,
  "default_connection": null,
  "default_section_frame_border": null,
  "default_focused_section_frame_border": null,
  "theme_variables": { "--bg": "#141414" },
  "theme_variants": { "dark": { "--bg": "#141414" }, "light": { "--bg": "#f5f1e8" } }
}
```

| Field | Type | Notes |
|---|---|---|
| `background_color` | string | `#RRGGBB` or `var(--name)` |
| `default_border` | object\|null | Fallback `GlyphBorderConfig` for every node |
| `default_connection` | object\|null | Fallback `GlyphConnectionConfig` for every edge |
| `default_section_frame_border` | object\|null | Fallback `GlyphBorderConfig` for unfocused section frames in NodeEdit mode. `null` → hardcoded `light`-preset thin line. See [`border-patterns.md`](./border-patterns.md#section-frames). |
| `default_focused_section_frame_border` | object\|null | Fallback for the focused section's frame (the section whose text editor is open). `null` → hardcoded `heavy`-preset thick line. |
| `theme_variables` | object | Live map from variable name (e.g. `--bg`) to color |
| `theme_variants` | object | Named theme presets; one is copied to `theme_variables` |

## Node

```json
{
  "id": "1.2",
  "parent_id": "1",
  "position": { "x": 0.0, "y": 0.0 },
  "size": { "width": 240.0, "height": 60.0 },
  "sections": [
    { "text": "Hello", "text_runs": [] }
  ],
  "style": { ... },
  "layout": { ... },
  "folded": false,
  "notes": "",
  "color_schema": null,
  "channel": 0,
  "trigger_bindings": [],
  "inline_mutations": []
}
```

| Field | Type | Notes |
|---|---|---|
| `id` | string | Dewey-decimal structural ID — see [ids.md](./ids.md) |
| `parent_id` | string\|null | Parent reference; `null` for roots |
| `position.x`, `position.y` | number | Absolute canvas coordinates. May be negative (nodes float freely on the canvas with no parent AABB constraint). |
| `size.width`, `size.height` | number | Pixel dimensions. Must be finite and strictly positive — `set_node_size` and `set_node_aabb` reject zero / negative / non-finite components, and `set_node_size` flags >100× the prior axis as a likely typo. The `node resize <w> <h>` console verb and the drag-to-resize gesture both route through these setters. |
| `sections` | array | The user-data strata of this node — one or more positioned text-bearing surfaces inside the node AABB. Every renderable node has at least one section. Pre-section maps (`text` / `text_runs` directly on the node) are migrated by `maptool convert --sections`. See [sections.md](./sections.md). |
| `style` | object | Visual styling (colors, shape, border) |
| `layout` | object | How this node's *children* are arranged |
| `folded` | bool | If `true`, hide the subtree below this node |
| `notes` | string | Free-form notes; empty string when none |
| `color_schema` | object\|null | Palette reference plus this node's own exceptions to it (`overrides`) — see [palettes.md](./palettes.md) |
| `channel` | integer | Mutation channel — see [channels.md](./channels.md); defaults to 0 |
| `trigger_bindings` | array | Event→mutation bindings attached to this node |
| `inline_mutations` | array | Node-local custom mutation definitions |
| `inline_macros` | array | Node-local macro definitions (`Inline` privilege tier — highest precedence). Same opaque-JSON shape and privilege model as `MindMap.macros`. Full reference: [`macros.md`](./macros.md). |
| `min_zoom_to_render` | number\|null | Lower bound on `camera.zoom` at which this node (and its glyph border) renders — see [zoom-bounds.md](./zoom-bounds.md). Inclusive; absent / `null` = unbounded below |
| `max_zoom_to_render` | number\|null | Upper bound on `camera.zoom` at which this node renders. Inclusive; absent / `null` = unbounded above |

## NodeStyle

```json
{
  "background_color": "#141414",
  "frame_color": "#30b082",
  "text_color": "#ffffff",
  "shape": "rectangle",
  "corner_radius_percent": 10.0,
  "frame_thickness": 4.0,
  "show_frame": true,
  "show_shadow": false,
  "border": null
}
```

| Field | Type | Notes |
|---|---|---|
| `background_color` | string | `#RRGGBB`, empty (`""` for transparent), or `var(--name)` |
| `frame_color` | string | Border color |
| `text_color` | string | Base text color — the section default, which a `TextRun`'s own `color` overrides |
| `shape` | string | See [enums.md](./enums.md) |
| `corner_radius_percent` | number | 0–100 |
| `frame_thickness` | number | Border width in pixels |
| `show_frame` | bool | Whether to render the border |
| `show_shadow` | bool | Whether to render a drop shadow |
| `border` | object\|null | `GlyphBorderConfig`; optional per-node override |

The three color fields are the **fallback** tier. A node carrying a
`color_schema` takes its fill, frame and text from the referenced
palette group instead — or from `color_schema.overrides` where that
node has excepted a channel by hand — and these values are what it
would fall back to if the schema were removed. They are *not* where
a per-node recolor lands on a themed node; that is
`color_schema.overrides`, because the read path never consults
`style` while a theme is bound. See [palettes.md](./palettes.md).

## NodeLayout

```json
{ "type": "map", "direction": "auto", "spacing": 50.0 }
```

| Field | Type | Values |
|---|---|---|
| `type` | string | `"map"`, `"tree"`, `"outline"` |
| `direction` | string | `"auto"`, `"up"`, `"down"`, `"left"`, `"right"`, `"balanced"` |
| `spacing` | number | Sibling gap in pixels |

## MindSection

```json
{
  "text": "Hello",
  "text_runs": [],
  "offset": { "x": 0.0, "y": 0.0 },
  "size": { "width": 240.0, "height": 60.0 },
  "channel": 0,
  "trigger_bindings": [],
  "frame_border": null
}
```

| Field | Type | Notes |
|---|---|---|
| `text` | string | Plain text content (may contain `\n`) |
| `text_runs` | array | Formatting spans inside this section — see [text-runs.md](./text-runs.md); defaults to empty |
| `offset.x`, `offset.y` | number | Top-left of the section AABB *relative to the owning node's `position`*, in canvas units. Defaults to `(0, 0)` (flush against the node's top-left). |
| `size` | object\|null | Section AABB. `null` (the default) means "fill the parent node"; an explicit width/height overrides. AABB containment uses the *effective* size (`null` ⇒ `node.size`), so a `null`-sized section is only valid at `offset = (0, 0)` — any non-zero offset stretches past the parent's right / bottom edge and `maptool verify` flags it. See [sections.md](./sections.md#effective-size-for-aabb-containment). |
| `channel` | integer | Mutation channel inside the parent node-area; defaults to the section's index in `MindNode.sections`. |
| `frame_border` | object\|null | Per-section [`GlyphBorderConfig`](#glyphborderconfig) override for the cyan rectangle drawn around the section while the owning node is in NodeEdit mode. `null` (the default) falls through to `Canvas.default_section_frame_border` (or the focused variant), then to a hardcoded thin / heavy preset. Same vocabulary node borders use, so any preset / pattern / corner / palette / font / size works. See [`border-patterns.md`](./border-patterns.md#section-frames). |
| `trigger_bindings` | array | Per-section [`TriggerBinding`s](./mutations.md). Section-level bindings fire *before* the whole-node bindings on `MindNode.trigger_bindings` — a section-targeted override (e.g. a different `OnClick` mutation per stratum of a multi-section node) takes precedence over catch-all node bindings. Defaults to empty. |

See [sections.md](./sections.md) for the section concept and
[text-runs.md](./text-runs.md) for the per-grapheme-run coverage rules.

## TextRun

```json
{
  "start": 0,
  "end": 5,
  "bold": true,
  "italic": false,
  "underline": false,
  "font": "LiberationSans",
  "size_pt": 14,
  "color": "#ffffff",
  "hyperlink": null
}
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `start` | integer | yes | First covered grapheme cluster (inclusive) |
| `end` | integer | yes | One past the last covered cluster (exclusive) |
| `bold` / `italic` / `underline` | bool | no | Default `false` |
| `font` | string | no | Family name; empty (the default) = no pin |
| `size_pt` | number | no | Fractional sizes valid; defaults to `14`, the run-less section scale |
| `color` | string | no | `#RRGGBB` or `var(--name)`; empty (the default) defers to the node's effective text color |
| `hyperlink` | string\|null | no | Absent / `null` = no link |

See [text-runs.md](./text-runs.md) for coverage rules and the default
semantics. Runs are addressed *relative to the owning section's
`text`* — there is no node-level `text_runs` after the section
refactor. Fields sitting at their defaults may be omitted when the
map is saved, per the omission policy above.

## ColorSchema (on a node)

```json
{
  "palette": "coral",
  "level": 0,
  "starts_at_root": true,
  "connections_colored": true
}
```

| Field | Type | Notes |
|---|---|---|
| `palette` | string | Key into `map.palettes` |
| `level` | non-negative integer | Depth from schema root; indexes `palette.groups`, clamped to the last group |
| `starts_at_root` | bool | `false` leaves the schema root on its own `style` and shifts the index down one |
| `connections_colored` | bool | Whether edges *leaving* this node take this node's frame tier — `overrides.frame`, else the group's `frame` — as their stroke |

`level` is a `usize`: a negative value is not a deep subtree, it is a
typo, and the loader rejects the file rather than silently clamping it
to the palette's last group (which is what the previous signed field
did, by way of an `as usize` wrap).

`overrides` is an optional object with the same four channel names
as a `ColorGroup` (`background`, `frame`, `text`, `title`), each an
optional `#RRGGBB` / `var(--name)` string. A channel present here is
this node's own color and outranks the group; the key is omitted
entirely when nothing is overridden, and an explicit `null` reads
the same as omitting it. It is where the interactive color setters
write on a themed node, and a `reset` on one of them drops the
channel back out.

An **empty** string is a hole rather than a color on `frame`,
`text` and `title`, exactly as it is in a `ColorGroup`: the reader
skips the tier. Only `background` carries one as a value, where it
means "no fill".

See [palettes.md](./palettes.md) for resolution semantics.

## Palette (on the map)

```json
{
  "groups": [
    { "background": "#a9decb", "frame": "#30b082", "text": "#000000", "title": "#000000" },
    { "background": "#f3b1c4", "frame": "#e24271", "text": "#000000", "title": "#000000" }
  ]
}
```

Each `ColorGroup` is `{ background, frame, text, title }` as `#RRGGBB`
strings. The `groups` array is indexed by the node's
`color_schema.level`. `background`, `frame` and `text` stand in for the
node's `style.background_color` / `frame_color` / `text_color`;
`title` stands in for `text` on the first line of the node's first
section.

An empty string means **"no palette value here"** in three of the four
channels — `frame`, `text` and `title` — and the node falls through to
the tier below (`style.frame_color`, `style.text_color`, and the text
role respectively). `background` is the exception and passes its empty
string through verbatim, because there empty is the format's "no fill,
let the canvas show through" spelling. The asymmetry is the point: a
transparent fill is a thing an author can want, while a transparent
frame is spelled `show_frame: false` and transparent text is not a
thing at all — an empty value in those channels reaches the color
parser and paints opaque black, discarding the `style` value it was
supposed to be shadowing.

## Edge

```json
{
  "from_id": "0",
  "to_id": "0.0",
  "type": "parent_child",
  "color": "#30b082",
  "width": 4,
  "line_style": "solid",
  "visible": true,
  "label": "optional label text",
  "anchor_from": "auto",
  "anchor_to": "auto",
  "control_points": []
}
```

| Field | Type | Notes |
|---|---|---|
| `from_id` | string | Source node ID |
| `to_id` | string | Target node ID |
| `type` | string | `"parent_child"` or `"cross_link"` |
| `color` | string | `#RRGGBB` or `var(--name)` |
| `width` | integer | Stroke width in pixels |
| `line_style` | string | See [enums.md](./enums.md) |
| `visible` | bool | Whether to render the edge |
| `label` | string\|null | Optional label text; omitted on save when unset |
| `label_config` | object\|null | Per-edge label position, color, and size-clamp overrides — see [`EdgeLabelConfig`](#edgelabelconfig) |
| `anchor_from` | string | Which side of the source node — see [enums.md](./enums.md) |
| `anchor_to` | string | Which side of the target node |
| `control_points` | array | Bezier offsets for curved edges |
| `glyph_connection` | object\|null | Per-edge glyph rendering override |
| `display_mode` | string\|null | `"line"` (default, absent) or `"portal"`. Portal-mode edges render as two glyph markers above each endpoint instead of a line; double-click a marker to jump to the other endpoint. |
| `min_zoom_to_render` | number\|null | Lower bound on `camera.zoom` at which this edge (body, caps, label, and portal endpoints unless overridden) renders — see [zoom-bounds.md](./zoom-bounds.md). Inclusive; absent / `null` = unbounded below |
| `max_zoom_to_render` | number\|null | Upper bound on `camera.zoom` at which this edge renders. Inclusive; absent / `null` = unbounded above |

### Portal-mode edges

Portal-mode edges use `display_mode = "portal"` and reuse
`glyph_connection.body` as the marker glyph, `edge.color` as the
marker color, and `glyph_connection.{font, font_size_pt}` for
typography. No separate portal struct — a portal is an edge
rendered differently.

```json
{
  "from_id": "0.3",
  "to_id": "1.7.2",
  "type": "cross_link",
  "color": "#30b082",
  "width": 3,
  "line_style": "solid",
  "visible": true,
  "anchor_from": "auto",
  "anchor_to": "auto",
  "control_points": [],
  "glyph_connection": { "body": "◈", "font_size_pt": 16.0 },
  "display_mode": "portal"
}
```

## GlyphBorderConfig

Optional per-node border rendered from font glyphs. All fields are
optional with defaults. The four side fields under `glyphs`
(`top`, `bottom`, `left`, `right`) are parsed as **side patterns**;
see [`border-patterns.md`](./border-patterns.md) for the grammar.

| Field | Type | Notes |
|---|---|---|
| `preset` | string | One of `"light"` (default), `"heavy"`, `"double"`, `"rounded"`, `"custom"`. The default's corner glyphs (`┌┐└┘`) extend to the cell edges so they connect cleanly with the side glyphs in monospace fonts; `"rounded"` (`╭╮╰╯`) curves inward and leaves a small visible gap at every corner — pick it deliberately if that's the intended look. |
| `font` | string\|null | Font family |
| `font_size_pt` | number | Glyph size |
| `color` | string\|null | `#RRGGBB`, falls back to `style.frame_color` |
| `glyphs` | object\|null | Custom glyphs when `preset == "custom"`. Sides (`top`, `bottom`, `left`, `right`) accept the [side-pattern grammar](./border-patterns.md); corners (`top_left`, `top_right`, `bottom_left`, `bottom_right`) are static glyph strings. |
| `padding` | number | Border-to-content padding in pixels |
| `color_palette` | string\|null | Optional palette name (key in top-level `palettes`) whose colors cycle per glyph around the border. When unset, every glyph paints in `color`. A missing palette name warns and falls back to the single-color path. |
| `color_palette_field` | string\|null | Which `ColorGroup` channel is cycled when `color_palette` is set: `"frame"` (default), `"background"`, `"text"`, or `"title"`. Unknown values warn and fall back to `"frame"`. |

## GlyphConnectionConfig

Optional per-edge connection rendering from repeated glyphs. All fields
optional.

| Field | Type | Default | Notes |
|---|---|---|---|
| `body` | string | `"·"` | Glyph repeated along the path |
| `cap_start` | string\|null | null | Glyph at the from-anchor |
| `cap_end` | string\|null | null | Glyph at the to-anchor (e.g. `"→"`) |
| `font` | string\|null | null | Font family |
| `font_size_pt` | number | 12.0 | Target on-screen size at zoom 1.0 |
| `min_font_size_pt` | number | 8.0 | Lower clamp |
| `max_font_size_pt` | number | 128.0 | Upper clamp |
| `color` | string\|null | null | Overrides `edge.color` when set |
| `spacing` | number | 0.0 | Gap between body glyphs |

## EdgeLabelConfig

Optional per-edge overrides for the text label that sits along a
line-mode edge's path. All fields are optional; an absent config
means "everything defaults" — the label sits at the path midpoint,
inherits the edge color, and sizes at `body_font_size × 1.1` with
the edge's clamps. Portal-mode edges ignore this; their per-endpoint
text styling lives on [`PortalEndpointState`](./portal-labels.md).

| Field | Type | Default | Notes |
|---|---|---|---|
| `position_t` | number\|null | 0.5 | Tangential position on the path, `[0.0, 1.0]` (from-anchor → to-anchor). |
| `perpendicular_offset` | number\|null | 0.0 | Signed canvas-unit offset along the path normal. Set by label drag. |
| `color` | string\|null | null | `#RRGGBB[AA]` or `var(--name)`. Cascades: own override → `glyph_connection.color` → `edge.color`. |
| `font_size_pt` | number\|null | `body × 1.1` | Target on-screen size at zoom 1.0. |
| `min_font_size_pt` | number\|null | inherits | Lower screen-space clamp. Falls back to the edge's `min_font_size_pt`. |
| `max_font_size_pt` | number\|null | inherits | Upper screen-space clamp. Falls back to the edge's `max_font_size_pt`. |
| `min_zoom_to_render` | number\|null | inherits | Lower zoom bound — replace-not-intersect cascade: when either this or `max_zoom_to_render` is set, **replaces** the parent edge's pair. Inclusive. See [zoom-bounds.md](./zoom-bounds.md). |
| `max_zoom_to_render` | number\|null | inherits | Upper zoom bound. Same cascade rule. Inclusive. |

## CustomMutation

```json
{
  "id": "switch-dark",
  "name": "Switch to dark theme",
  "description": "Copy the 'dark' theme variant into live variables.",
  "contexts": ["map.node"],
  "target_scope": "SelfOnly",
  "document_actions": [ { "SetThemeVariant": "dark" } ]
}
```

Map-level custom mutations are referenced by `TriggerBinding.mutation_id`
on a node, dispatched by `OnClick` / `OnHover` / `OnKey` triggers, or
applied explicitly via the `mutation apply <id>` console verb.

See [`mutations.md`](./mutations.md) for the complete reference:
four-source loader (app / user / map / inline), the `contexts`
namespace (`internal`, `map`, `map.node`, `map.tree`), the `mutator`
MutatorNode AST (used for declarative mutators with runtime holes),
and the imperative `DynamicMutationHandler` seam (used for
size-aware layouts like `flower-layout` / `tree-cascade`).

`target_scope` is one of `"SelfOnly"`, `"Children"`,
`"Descendants"`, `"SelfAndDescendants"`, `"Parent"`, `"Siblings"`,
`"SectionsOnly"`.
`behavior` defaults to `"Persistent"`; `"Toggle"` reverses on
second trigger.

## TriggerBinding (on a node)

```json
{ "trigger": "OnClick", "mutation_id": "switch-dark" }
```

Trigger is one of `"OnClick"`, `"OnHover"`, `"OnKey"`, `"OnLink"`.
