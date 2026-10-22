# `MindMap.canvas`

The map-wide defaults that every node and section falls back to
when its own field is unset. Authoring is through the `canvas …`
console verb; the data model lives at
`baumhard::mindmap::model::Canvas`.

## Field reference

| Field | Type | Default | Notes |
|---|---|---|---|
| `theme_variables` | object | `{}` | Map-wide CSS-variable-style color palette. Every `var(--name)` reference in node / edge / section styles resolves through this table. Authoring via `theme set <name> <value>`. |
| `palettes` | object | `{}` | Named color palettes, serving two consumers: a node's `color_schema` resolves its fill / frame / text / title through one (see [palettes.md](./palettes.md)), and every `color_palette = "…"` reference (on a node / section / border) cycles one per glyph. Authoring via `palette …` console verbs. Note this key lives on the **map**, not inside `canvas` — it is listed here because it is map-wide shared state. |
| `default_border` | object\|null | `null` | The map-wide border every framed node falls back to. Same shape as `MindNode.style.border`. Authoring via `canvas border …`. |
| `default_section_frame_border` | object\|null | `null` | The map-wide default for the cyan rectangle drawn around an unfocused section in NodeEdit mode. Same shape as `MindNode.style.border`. Authoring via `canvas section-frame …`. |
| `default_focused_section_frame_border` | object\|null | `null` | The map-wide default for the focused section's frame (the section currently being text-edited). Same shape; falls back to `default_section_frame_border` then to a hardcoded heavy floor. Authoring via `canvas section-frame focused …`. |

## Cascade resolution

Every renderable property cascades:

```
per-node / per-section override   ← authored via `border …` / `section frame …`
  ↓ (if unset)
canvas default                    ← authored via `canvas border …` / `canvas section-frame …`
  ↓ (if unset)
hardcoded floor                   ← `light` preset for borders, `heavy` for focused section frames
```

The same cascade discipline applies to `theme_variables` /
`palettes` lookups: a `var(--accent)` reference fails through to a
hardcoded warn-and-fall-back if the canvas table doesn't define
the name.

## Console verbs

See [`border-patterns.md`](./border-patterns.md)'s "Console verb"
section for the full `canvas border …` and
`canvas section-frame [focused] …` grammar (positional + kv
forms, preview lifecycle, cycle / toggle subverbs).

`canvas border preset cycle` and the per-section
`section frame preset=…` parity work landed in Batch 6 / Batch 8;
canvas-side `show side=` filter and `verbose` flag are open
follow-ups (the canvas show formatter has a different shape
without per-node `size` to draw a side-rendered preview from).

## Schema migration note

The three `default_*` border fields landed in the
section-borders-resize PR (Batches 2 / 5 / 6). All default to
`null`, so legacy `.mindmap.json` files load unchanged (serde
defaults absorb absent fields). No schema-version bump.

The `MindSection.frame_border` per-section override landed in the
same PR — see [`sections.md`](./sections.md#field-reference) for
its row in the section field table.
