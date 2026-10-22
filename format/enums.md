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
"rectangle", "rounded_rectangle", "ellipse", "circle", "diamond",
"parallelogram", "hexagon"
```

Inherited from miMind's shape catalog (plus the `"circle"` alias).
Default: `"rectangle"`.

`"rectangle"` and `"ellipse"` are **live shapes** — they drive both the
background fill (SDF in the rect pipeline) and the hit test (BVH descent
plus `rect_select`) via
[`NodeShape`](../lib/baumhard/src/gfx_structs/shape.rs). A perfect circle
is expressed as `"ellipse"` with `width == height`; stretched /
"conical" cases are the same variant with `width != height`. `"circle"`
is accepted as an alias for `"ellipse"` — the convenience spelling
authors reach for first.

The remaining values (`"rounded_rectangle"`, `"diamond"`,
`"parallelogram"`, `"hexagon"`) still round-trip but currently render
as `"rectangle"` — adding one is a small change (one `NodeShape`
variant, one `style_spellings` arm, one WGSL `case`, one
`contains_local` arm; see the header doc in `shape.rs`).

Falling back is **not** an error for any of the values listed above:
they are canonical, `maptool verify` accepts them, and
`maptool convert --legacy` emits them, so the runtime substitutes a
rectangle at `log::trace!` and says nothing in release. A spelling
that is *not* in the list — a typo, or a value from a newer build —
falls back the same way but emits a `log::warn!` naming it. The
classifier behind that split is `ShapeSpelling` in `shape.rs`, and the
list it consults is the `KNOWN_SHAPES` constant there — the only list
any parser reads. Adding a spelling to that constant makes it
non-warning at load time and valid under `maptool verify` with no
parser change.

**Five restatements** of `KNOWN_SHAPES` exist, across three files, and
none of them is trusted:

| # | Restatement | Pinned by |
| --- | --- | --- |
| 1 | the fence above | `test_shape_format_doc_publishes_exactly_known_shapes` |
| 2 | the live-shapes sentence | the same test |
| 3 | the "remaining values" parenthesis | the same test |
| 4 | `LEGACY_SHAPE_ORDINALS` in `crates/maptool/src/convert/enums.rs` | `legacy_shape_ordinals_are_canonical_spellings` |
| 5 | the `shape_type` ordinal table in [migration.md](./migration.md#mimind-shape_type-ordinals) | `legacy_shape_ordinals_match_the_published_table`, against row 4 |

Baumhard's `test_shape_format_doc_publishes_exactly_known_shapes`
reads this section back off disk and checks rows 1–3 against the
constant, deriving each expectation from the code rather than
restating it; maptool's two tests do the same for the converter and
for the published numbering. So a spelling added to `KNOWN_SHAPES` and
not to this section fails the first test, and a rename that leaves the
converter emitting the old spelling — which would make every converted
node warn on load and `maptool verify` reject a file this repo's own
tool just wrote — fails the second. Row 5 is pinned to row 4 rather
than to the constant directly; since row 4 is pinned to the constant,
the chain still closes.

Today, `show_frame = true` only emits the glyph border when
`shape == "rectangle"` — the eight-run frame layout assumes an
axis-aligned box. Non-rectangular nodes drop the frame until a
shape-aware border lands.

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
