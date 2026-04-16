# Palettes

Color schemas are defined **once** at the map level and **referenced** by
name from each themed node.

```json
{
  "palettes": {
    "coral": {
      "groups": [
        { "background": "#a9decb", "frame": "#30b082", "text": "#000000", "title": "#000000" },
        { "background": "#f3b1c4", "frame": "#e24271", "text": "#000000", "title": "#000000" }
      ]
    }
  },
  "nodes": {
    "0": {
      "color_schema": {
        "palette": "coral",
        "level": 0,
        "starts_at_root": true,
        "connections_colored": true
      }
    }
  }
}
```

## Why hoist palettes to the map level?

The legacy format stored the full palette definition — every color group,
for every depth level — **on every themed node**. In the testament map,
243 of 243 nodes carried a `color_schema` object; 225 of those duplicated
the palette name, variant code, theme_id, and an empty `groups` array;
only 18 (the schema roots) actually contained the palette data.

Every child node copied fields that only made sense on the root, because
the legacy renderer walked up the parent chain to find the definition.
The file was twenty-three megabytes of redundancy.

Hoisting palettes solves this:

- Each palette is defined once, in one place
- Editing a palette is a single-point change; every node using it
  updates on the next render
- Per-node `color_schema` becomes tiny: a palette name, a depth level,
  two flags
- User-defined palettes and palette-switching mutations become natural
  (palette is a named thing you can reference, not a blob buried in a
  node's data)

## How a node resolves its colors

`resolve_theme_colors(node)` in `lib/baumhard/src/mindmap/model/mod.rs`:

1. Read `node.color_schema` — if absent, the node uses the colors in its
   `style` (background_color, frame_color, text_color)
2. Look up `schema.palette` in `map.palettes`
3. Index into `palette.groups` by `schema.level`. If the level exceeds
   the group count, clamp to the last group.

If the palette name doesn't exist in the map, `resolve_theme_colors`
returns `None` and the renderer falls back to the node's plain `style`
colors. `maptool verify` flags missing palette references as errors.

## What `level` means

Depth from the schema root. The root of a themed subtree has `level: 0`
and indexes into `groups[0]`. Its children have `level: 1` (groups[1]),
grandchildren `level: 2`, and so on. A palette with 7 groups themes 7
levels of hierarchy before wrapping.

`level` is stored explicitly rather than computed from parent chain depth
because subtrees may be themed independently — a deep subtree can restart
at level 0 with a different palette.

## The `starts_at_root` and `connections_colored` flags

Inherited from miMind. `starts_at_root` controls whether the palette's
level 0 applies to the root of the themed subtree or to its children.
`connections_colored` controls whether edges inherit palette colors for
their stroke.

Both are preserved faithfully; see `schema.md` for the full semantic.

## What's no longer in the format

The legacy per-node color schema carried three fields that the new format
drops:

- `groups`: now lives on the palette, not the node
- `theme_id` (e.g. `"Pastel:#BFFFFFFE01"`): an opaque miMind-internal
  identifier; Mandala never read it
- `variant`: an integer variant code; if a map had multiple variants of
  the same palette name, the converter folds the variant into the palette
  name (`"coral"` vs `"coral-v3"`)

`maptool convert --legacy` performs the hoisting automatically.
