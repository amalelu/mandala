# Named Enums

Fields that take a closed set of values use **named strings** rather than
integer codes.

```json
"style": { "shape": "rectangle" }
"layout": { "type": "map", "direction": "auto" }
"edge": { "line_style": "solid", "anchor_from": "top", "anchor_to": "bottom" }
```

## Why named strings over integers?

The legacy format inherited miMind's Java enum ordinals: `shape_type: 0`,
`layout.type: 1`, `anchor_from: 3`. Opaque integers meant:

- You couldn't read a file and tell what was rendered
- Every caller needed a lookup table to interpret values
- Typos (`shape_type: 5` when you meant 4) silently produced garbage
  instead of an error

Named strings make the file self-documenting. `"rectangle"` means
rectangle; `"bottom"` means the bottom anchor. Reviewing a map's JSON
becomes a prose exercise, not a decoding one.

## The known values

### `style.shape`

```
"rectangle", "rounded_rectangle", "ellipse", "diamond",
"parallelogram", "hexagon"
```

Inherited from miMind's shape catalog. The current Mandala renderer uses
glyph-based borders (`style.border`), so `shape` is primarily preservation
for round-trip fidelity and future export targets. Default: `"rectangle"`.

### `layout.type`

```
"map"       — free 2D placement
"tree"      — structured, fixed distance on branch axis
"outline"   — linear list at fixed distance
```

Controls how a node's **children** are arranged, not the node itself.

### `layout.direction`

```
"auto", "up", "down", "left", "right", "balanced"
```

Paired with `layout.type`. `"balanced"` means radial scatter with left/right
parity; `"auto"` lets the layout pick based on context.

### `edge.line_style`

```
"solid", "dashed"
```

Currently two values. The string form is extensible — adding `"dotted"`
in the future is a format addition, not a breaking change. Integer codes
would have had no graceful forward-compat story.

### `edge.anchor_from` / `edge.anchor_to`

```
"auto", "top", "right", "bottom", "left"
```

Which edge midpoint on the node's bounding box a connection attaches to.
`"auto"` picks the midpoint closest to the other endpoint.

`resolve_anchor_point` in
`lib/baumhard/src/mindmap/connection/mod.rs` matches on these strings
directly — the enum values flow from format to renderer without an
intermediate lookup table.

### `edge.type`

```
"parent_child", "cross_link"
```

`parent_child` edges render the hierarchical connection between a node and
its parent (one per non-root node). `cross_link` edges are arbitrary
non-hierarchical connections between any two nodes.

### `edge.display_mode`

```
"line" (default, absent in JSON), "portal"
```

`"line"` renders the edge as the usual path between endpoints.
`"portal"` renders the edge as two glyph labels — one attached to
each endpoint node's border — without a line between them.
Portal-mode edges are orthogonal to `edge.type`: either
`parent_child` or `cross_link` can be rendered as a portal.
Single-clicking a portal label selects that specific label for
color / copy / paste / cut operations (see `portal-labels.md` for
the per-endpoint state). Double-clicking navigates the camera to
the opposite endpoint. Dragging a label along the node's border
pins it to a user-chosen position; without a drag, each label
auto-orients toward its partner endpoint.

## Unknown values

Renderer code matches on the known strings with a default fallback. An
unexpected `shape: "oblong"` renders as `"rectangle"` (the default), not
an error — keeping the file loadable.

`maptool verify` **does** flag unknown enum values so authors catch typos
before they render silently wrong.

## Forward compatibility

New enum values can be added to the format without breaking old readers
(they fall back to defaults) and without breaking old writers (they keep
emitting the values they know). No version bump needed. This is the main
reason named strings win over integer codes here — you can grow the
vocabulary without coordinating across the codebase.
