# Schema Reference

Complete field reference for every type in `.mindmap.json`.

## Unknown keys are rejected

Every object in a `.mindmap.json` is **closed**. A key that no field
claims is a load error, not a field the loader quietly ignores.

The reason lives on the *save* path, not the load path. Mandala is an
editor: it loads the whole map, mutates it, and writes the whole model
back. A key dropped at load is therefore a key **deleted from the file
at the next save** — and for a hand-authored map, that file was the
only copy. A typo (`"min_zoom_to_rendr": 2.0`) and a field that does
not exist yet (`"portal_form": { … }`) fail the same way, and both
fail while the author still has what they wrote. The IPC boundary made
the same call for the same reason: see [`ipc.md`](./ipc.md), "unknown
parameters are rejected with `invalid_params`".

This map does **not** load:

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

```
node "0": unknown field `min_zoom_to_rendr`, expected one of `id`,
`parent_id`, `position`, `size`, `sections`, `style`, `layout`,
`folded`, `notes`, `color_schema`, `channel`, `trigger_bindings`,
`inline_mutations`, `inline_macros`, `min_zoom_to_render`,
`max_zoom_to_render` — unknown keys are rejected, not dropped: a load
that ignored this key would erase it from the file on the next save.
See format/schema.md.
```

Fix the spelling and it loads:

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

The converse is not a loss: a key written out at its own default
value (`"color_schema": null`, `"text_runs": []`) may be *omitted*
when the map is saved. Nothing that carries information disappears —
reload the saved file and the model is the same one.

The message names the **part of the document** that carries the key —
`node "1.2"`, `edge[3]`, `palette "coral"`, `canvas`,
`custom_mutations[0]` — rather than a byte offset, because a map runs
to thousands of lines and the key is the thing you can search for.

Two shapes get a better message than the generic one, because "unknown
field `text`" is true but useless to someone holding a pre-refactor
map: a top-level `portals` array and per-node `text` / `text_runs` are
answered with the `maptool convert` verb that migrates them. See
[`migration.md`](./migration.md).

**What this does not cover.** Closedness is about *keys*, not
*meanings*. An edge whose `to_id` names no node, a `color_schema`
pointing at a palette that isn't there, a section that hangs outside
its node — all of those are spelled correctly and all of them load.
They are `maptool verify`'s job; see [`validation.md`](./validation.md).

Values inside `macros` and a node's `inline_macros` are the deliberate
exception: baumhard stores them as opaque JSON (the typed `Macro` lives
in the application crate), so their interiors are carried through
untouched rather than checked here. See [`macros.md`](./macros.md).

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
| `color_schema` | object\|null | Palette reference — see [palettes.md](./palettes.md) |
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
| `text_color` | string | Base text color |
| `shape` | string | See [enums.md](./enums.md) |
| `corner_radius_percent` | number | 0–100 |
| `frame_thickness` | number | Border width in pixels |
| `show_frame` | bool | Whether to render the border |
| `show_shadow` | bool | Whether to render a drop shadow |
| `border` | object\|null | `GlyphBorderConfig`; optional per-node override |

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

See [text-runs.md](./text-runs.md) for coverage rules. Runs are
addressed *relative to the owning section's `text`* — there is no
node-level `text_runs` after the section refactor.

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
| `level` | integer | Depth from schema root; indexes `palette.groups` |
| `starts_at_root` | bool | Whether level-0 applies to the root or its children |
| `connections_colored` | bool | Whether edges inherit palette colors |

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
strings. The `groups` array is indexed by the node's `color_schema.level`.

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
  "label": null,
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
| `label` | string\|null | Optional label text |
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
  "label": null,
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
