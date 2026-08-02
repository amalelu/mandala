# Validation

A file that serde can deserialize is **syntactically** valid but might
still be **semantically** broken: dangling edge references, parent_id
pointing to a nonexistent node, Dewey IDs that disagree with `parent_id`,
palette references that don't resolve. These are caught by:

```
maptool verify <map.json>
```

Exit code 0 if clean, or if only warnings are found. Nonzero with a
list of violations if any error is found. Warnings are still printed so
a CI recipe that captures stderr can see them.

The split is between *spelling* and *meaning*. Spelling is the
loader's: every object in the format is closed, so a key no field
claims fails the load outright rather than being dropped and then
erased at the next save — see [schema.md](./schema.md#unknown-keys-are-rejected).
Everything below is about correctly-spelled files that still say
something incoherent, and that is `verify`'s.

## What gets checked

### Tree structure

- Every non-null `parent_id` points to a node that exists in `nodes`
- No cycles in the `parent_id` chain

**Why**: a node whose parent doesn't exist is unreachable through tree
traversal. A cycle makes `all_descendants` loop forever.

### ID consistency

- The HashMap key equals `node.id` for every entry
- For every non-root node, `derive_parent_id(node.id)` agrees with
  `node.parent_id`
- Root nodes (`parent_id: null`) have no dot in their ID

**Why**: the Dewey ID encodes structure. If `"1.2"` claims its parent is
`"0"`, either the ID is lying or `parent_id` is — which one loads right
depends on which code path runs, a reliability nightmare.

### References

- Every edge's `from_id` and `to_id` exist in `nodes`
- No two edges share the same `(from_id, to_id, edge_type)` tuple

**Why**: dangling references silently disappear at render time — the
connection just doesn't draw, with no indication that something was
lost. Applies uniformly to line-mode and portal-mode edges.

Duplicate tuples break edge identity: `EdgeRef` lookups return the
first match, `SceneConnectionCache` overwrites with the second edge's
geometry, and the on-disk format no longer round-trips through the
runtime faithfully. Multiple edges between the same pair with
different `edge_type` values are allowed.

### Palettes

- Every node with `color_schema` references a palette that exists in
  `map.palettes`
- Every palette has at least one group

**Why**: a missing palette falls back to the node's base `style` colors,
which silently wipes the theme. An empty palette produces no colors at
any level.

### Named enums

- `style.shape` is one of `"rectangle"`, `"rounded_rectangle"`,
  `"ellipse"`, `"circle"`, `"diamond"`, `"parallelogram"`, `"hexagon"`
  (compared case-insensitively)
- `layout.type` is one of `"map"`, `"tree"`, `"outline"`
- `layout.direction` is one of `"auto"`, `"up"`, `"down"`, `"left"`,
  `"right"`, `"balanced"`
- `edge.line_style` is one of `"solid"`, `"dashed"`
- `edge.anchor_from` and `anchor_to` are one of `"auto"`, `"top"`,
  `"right"`, `"bottom"`, `"left"`
- `edge.type` is one of `"parent_child"`, `"cross_link"`
- `edge.display_mode` (if present) is one of `"line"`, `"portal"`

See [enums.md](./enums.md) for the complete lists.

**Why**: the renderer falls back to defaults on unknown values. An author
typo (`shape: "retcangle"`) silently becomes a plain rectangle. Verify
catches the typo.

### Text runs

- Runs do not overlap (which implies ascending `start` order for
  well-formed runs)
- Each run's `start < end`
- `end` is within the text's grapheme-cluster count

**Why**: overlapping or out-of-bounds runs produce undefined rendering —
the first run wins silently, the tail is clipped. Rich text bugs are
painful to diagnose after the fact.

### Section bounds

For every `MindNode.sections[i]`:

- `offset.{x,y}` finite and non-negative
- The section's effective AABB (`offset + effective_size`) is inside
  the parent node's `size`. The effective size is the explicit
  `section.size` when set, otherwise `node.size` (fill-parent).
- `size.{width,height}` (when set) finite and strictly positive, and
  not over 100× the parent's matching dimension (typo guard)
- The owning `node.size.{width,height}` itself finite, strictly
  positive, and not over `MAX_NODE_AXIS` (`1_000_000.0`) (sub-check,
  since a corrupt node-size cascades into every section's AABB math)
- `node.sections.len()` does not exceed `MAX_SECTIONS_PER_NODE`
  (`1024`)
- No two sections share the same effective channel under the same
  parent — the effective channel is `section.channel.unwrap_or(section_idx)`.
  Surfaced as a *warning*, not a hard rejection: the broadcast
  is well-defined (a mutation targeting the shared channel hits
  every listed section), and authors who deliberately want
  broadcast can ignore it. Most cases are typos.

**Why**: an out-of-bounds section silently overflows its parent
container at render time and breaks hit-testing; a NaN at the
node level poisons every downstream AABB comparison. The console
verbs `section move dx=<dx> dy=<dy>` and `section resize w=<w> h=<h>`
([sections.md](./sections.md)) enforce these same rules at edit
time and surface byte-equal rejection messages — a verb-rejected
edit and a `verify` violation read identically.

### Zoom bounds

- `min_zoom_to_render` and `max_zoom_to_render` (when set) are finite
- Whenever both are set on a `MindNode`, `MindEdge`,
  `EdgeLabelConfig`, or `PortalEndpointState`, `min <= max` holds

**Why**: an inverted pair is a well-defined but always-invisible
window — the render-time check still terminates cleanly, but an
element that never renders at any zoom is almost always a typo. See
[zoom-bounds.md](./zoom-bounds.md).

## What's not checked

- **Color format** (`#RRGGBB` vs `rgb(...)` vs named colors): the format
  says hex or `var(--name)`, but the renderer is lenient. We don't verify
  color syntax — authors who type `"red"` will see default colors, and
  that's easy to diagnose visually.
- **Positions and sizes**: negative positions are valid (the canvas is
  unbounded). Zero-size nodes are rare but not forbidden.
- **ID stability after reparent**: Dewey IDs can drift from parent_id
  after a runtime reparent (documented in [ids.md](./ids.md)). Verify
  **does** flag ID/parent_id mismatches — saving a reparented map and
  running verify will report the drift as a violation. This is
  intentional: the on-disk format should be consistent, even if the
  runtime allows transient drift.
- **Referential integrity of `trigger_bindings.mutation_id`**: if a
  binding references a mutation ID that doesn't exist, the binding is a
  no-op at runtime. Verify could be extended to flag this; currently it
  doesn't.

## Running verify in CI

`maptool verify` exits 0 on success, nonzero on violations. A CI job that
verifies every `.mindmap.json` in the repo is a natural safety net:

```bash
for f in maps/*.mindmap.json; do
  maptool verify "$f" || exit 1
done
```

## Violation output format

Errors:

```
<category> @ <location>: <message>
```

Warnings are printed the same way but prefixed with `warning:`:

```
warning: sections @ 0: channel 0 shared by sections [0, 1]; ...
```

Examples:

```
tree @ 1.2: parent_id "9.9" references a node that does not exist
ids @ 1.2.3: parent_id "1.0" does not match derived parent "1.2"
references @ edge[0]: from_id "5.5" is not a node
edges @ edge[3]: duplicate edge (from_id="0", to_id="1", type="cross_link") first seen at edge[0]
palettes @ 0: palette "sunset" is not defined in map.palettes
enums @ 0: style.shape "oblong" is not a known shape
text_runs @ 0: section[0].run[1] overlaps previous run (start 3 < previous end 5)
zoom_bounds @ edge[0]: min_zoom_to_render 2 > max_zoom_to_render 0.5
```

Each violation names its category, the location inside the file, and what
went wrong. The location format varies by category (node ID, edge index,
etc.) but is always clickable / greppable.
