# MindMap Format

This directory documents the `.mindmap.json` format — the on-disk
representation of a Mandala mindmap. The format is designed to feel native
to the application, not as a converted import from an external tool.

## Start here

- [`schema.md`](./schema.md) — complete field reference for every type
- [`validation.md`](./validation.md) — the invariants a valid file must
  satisfy (and what `maptool verify` checks)

## Concept explainers

Each of these documents one decision, with the reasoning:

- [`ids.md`](./ids.md) — Dewey-decimal structural IDs
- [`palettes.md`](./palettes.md) — map-level palette definitions
- [`channels.md`](./channels.md) — the `channel` field and mutation targeting
- [`enums.md`](./enums.md) — named string enums over integer codes
- [`text-runs.md`](./text-runs.md) — rich text formatting
- [`zoom-bounds.md`](./zoom-bounds.md) — per-element
  `min_zoom_to_render` / `max_zoom_to_render` window gating render
  presence on camera zoom
- [`mutations.md`](./mutations.md) — `CustomMutation` carrier,
  four-source loader, the `mutation` console verb, and the contexts
  taxonomy

## Tools

- [`migration.md`](./migration.md) — converting legacy miMind-derived files
  with `maptool convert --legacy`

## Minimum-viable example

```json
{
  "version": "1.0",
  "name": "hello",
  "canvas": { "background_color": "#000000" },
  "nodes": {
    "0": {
      "id": "0",
      "parent_id": null,
      "position": { "x": 0.0, "y": 0.0 },
      "size": { "width": 200.0, "height": 60.0 },
      "text": "Hello",
      "text_runs": [],
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
      "channel": 0
    }
  },
  "edges": []
}
```

That's a complete, valid mindmap with a single root node. Everything beyond
this is optional or additive.
