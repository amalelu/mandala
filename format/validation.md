# Validation

A file that serde can deserialize is **syntactically** valid but might
still be **semantically** broken: dangling edge references, parent_id
pointing to a nonexistent node, Dewey IDs that disagree with `parent_id`,
palette references that don't resolve. These are caught by:

```
maptool verify <map.json>
```

Exit code 0 if clean. Nonzero with a list of violations if anything is
off.

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
- Every portal's `endpoint_a` and `endpoint_b` exist in `nodes`
- Portal labels are unique within the map

**Why**: dangling references silently disappear at render time — the
connection or portal just doesn't draw, with no indication that something
was lost.

### Palettes

- Every node with `color_schema` references a palette that exists in
  `map.palettes`
- Every palette has at least one group

**Why**: a missing palette falls back to the node's base `style` colors,
which silently wipes the theme. An empty palette produces no colors at
any level.

### Named enums

- `style.shape` is one of the known shape values
- `layout.type` is one of `"map"`, `"tree"`, `"outline"`
- `layout.direction` is one of `"auto"`, `"up"`, `"down"`, `"left"`,
  `"right"`, `"balanced"`
- `edge.line_style` is one of `"solid"`, `"dashed"`
- `edge.anchor_from` and `anchor_to` are one of `"auto"`, `"top"`,
  `"right"`, `"bottom"`, `"left"`
- `edge.type` is one of `"parent_child"`, `"cross_link"`

See [enums.md](./enums.md) for the complete lists.

**Why**: the renderer falls back to defaults on unknown values. An author
typo (`shape: "retcangle"`) silently becomes a plain rectangle. Verify
catches the typo.

### Text runs

- If non-empty, runs are ordered by ascending `start`
- Runs do not overlap
- Each run's `start < end`
- `end` is within the text's code-point count

**Why**: overlapping or out-of-bounds runs produce undefined rendering —
the first run wins silently, the tail is clipped. Rich text bugs are
painful to diagnose after the fact.

## What's not checked

- **Color format** (`#RRGGBB` vs `rgb(...)` vs named colors): the format
  says hex or `var(--name)`, but the renderer is lenient. We don't verify
  color syntax — authors who type `"red"` will see default colors, and
  that's easy to diagnose visually.
- **Positions and sizes**: negative positions are valid (the canvas is
  unbounded). Zero-size nodes are rare but not forbidden.
- **ID stability after reparent**: Dewey IDs can drift from parent_id
  after a runtime reparent (documented in [ids.md](./ids.md)). Verify
  accepts the drift — it's expected.
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

```
<category> @ <location>: <message>
```

Example:

```
tree @ 1.2: parent_id "9.9" references a node that does not exist
ids @ 1.2.3: parent_id "1.0" does not match derived parent "1.2"
references @ edge[0]: from_id "5.5" is not a node
palettes @ 0: palette "sunset" is not defined in map.palettes
enums @ 0: style.shape "oblong" is not a known shape
text_runs @ 0: run[1] overlaps run[0] (start 3 < previous end 5)
```

Each violation names its category, the location inside the file, and what
went wrong. The location format varies by category (node ID, edge index,
etc.) but is always clickable / greppable.
