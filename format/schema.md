# Schema Reference

Complete field reference for every type in `.mindmap.json`.

## Top-level object

```json
{
  "version": "1.0",
  "name": "map-name",
  "canvas": { ... },
  "palettes": { ... },
  "nodes": { ... },
  "edges": [ ... ],
  "custom_mutations": [ ... ]
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

## Canvas

```json
{
  "background_color": "#000000",
  "default_border": null,
  "default_connection": null,
  "theme_variables": { "--bg": "#141414" },
  "theme_variants": { "dark": { "--bg": "#141414" }, "light": { "--bg": "#f5f1e8" } }
}
```

| Field | Type | Notes |
|---|---|---|
| `background_color` | string | `#RRGGBB` or `var(--name)` |
| `default_border` | object\|null | Fallback `GlyphBorderConfig` for every node |
| `default_connection` | object\|null | Fallback `GlyphConnectionConfig` for every edge |
| `theme_variables` | object | Live map from variable name (e.g. `--bg`) to color |
| `theme_variants` | object | Named theme presets; one is copied to `theme_variables` |

## Node

```json
{
  "id": "1.2",
  "parent_id": "1",
  "position": { "x": 0.0, "y": 0.0 },
  "size": { "width": 240.0, "height": 60.0 },
  "text": "Hello",
  "text_runs": [],
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
| `position.x`, `position.y` | number | Absolute canvas coordinates |
| `size.width`, `size.height` | number | Pixel dimensions |
| `text` | string | Plain text content (may contain `\n`) |
| `text_runs` | array | Formatting spans — see [text-runs.md](./text-runs.md); defaults to empty |
| `style` | object | Visual styling (colors, shape, border) |
| `layout` | object | How this node's *children* are arranged |
| `folded` | bool | If `true`, hide the subtree below this node |
| `notes` | string | Free-form notes; empty string when none |
| `color_schema` | object\|null | Palette reference — see [palettes.md](./palettes.md) |
| `channel` | integer | Mutation channel — see [channels.md](./channels.md); defaults to 0 |
| `trigger_bindings` | array | Event→mutation bindings attached to this node |
| `inline_mutations` | array | Node-local custom mutation definitions |

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

See [text-runs.md](./text-runs.md) for coverage rules.

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
| `label_position_t` | number\|null | Parameter-space label position (0.0–1.0) |
| `anchor_from` | string | Which side of the source node — see [enums.md](./enums.md) |
| `anchor_to` | string | Which side of the target node |
| `control_points` | array | Bezier offsets for curved edges |
| `glyph_connection` | object\|null | Per-edge glyph rendering override |
| `display_mode` | string\|null | `"line"` (default, absent) or `"portal"`. Portal-mode edges render as two glyph markers above each endpoint instead of a line; double-click a marker to jump to the other endpoint. |

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
optional with defaults.

| Field | Type | Notes |
|---|---|---|
| `preset` | string | `"light"`, `"heavy"`, `"double"`, `"rounded"`, `"custom"` |
| `font` | string\|null | Font family |
| `font_size_pt` | number | Glyph size |
| `color` | string\|null | `#RRGGBB`, falls back to `style.frame_color` |
| `glyphs` | object\|null | Custom glyphs when `preset == "custom"` |
| `padding` | number | Border-to-content padding in pixels |

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
| `max_font_size_pt` | number | 24.0 | Upper clamp |
| `color` | string\|null | null | Overrides `edge.color` when set |
| `spacing` | number | 0.0 | Gap between body glyphs |

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
`"Descendants"`, `"SelfAndDescendants"`, `"Parent"`, `"Siblings"`.
`behavior` defaults to `"Persistent"`; `"Toggle"` reverses on
second trigger.

## TriggerBinding (on a node)

```json
{ "trigger": "OnClick", "mutation_id": "switch-dark" }
```

Trigger is one of `"OnClick"`, `"OnHover"`, `"OnKey"`, `"OnLink"`.
