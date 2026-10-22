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

The split is three-way, not two, and the first column is the one
that moved.

1. **Spelling — `verify` reports.** It used to be the loader's: every
   object was closed, and a key no field claimed failed the load. It
   is not any more, and the reason matters — a key no field claims is
   now **kept**, so an older build can open a newer map without losing
   what it does not understand. See
   [schema.md](./schema.md#unknown-keys-are-kept). The loader warns
   once, into a log, and carries on. A warning at load is not a moment
   at which anybody finds out; a nonzero exit is. So an unrecognized
   key is an **error** under `verify`, reported at the part of the
   document that carries it, even though the map loads and renders
   fine.
2. **Render safety — the loader refuses.** A correctly-spelled file
   can still carry numbers that take the editor down rather than
   merely looking wrong: a zero font size trips an assert inside the
   text shaper, a crossed size window used to reach an `f32::clamp`,
   and node geometry sizes real allocations. Those are rejected at
   load, because a map that would abort the process must not open.
   The rules are listed under [Numeric domain](#numeric-domain)
   below, and the reasoning is in
   [macros.md](./macros.md#the-load-time-trust-boundary).
   **Parent cycles and the key-equals-`node.id` rule are in this
   column too** — both are load-blocking, because the cycle check
   and the scene builder address nodes by different spellings, and a
   disagreement between them is a walk that never terminates.
3. **Coherence — `verify` reports.** Everything else: a file that
   loads and renders but still says something incoherent. Dangling
   edge references, a Dewey ID that disagrees with `parent_id`, an
   unresolved palette name, a section hanging past its node's edge.

Columns 1 and 3 are both `verify`'s, and they differ in what they
say rather than in who says it: one is *this file is misspelled*, the
other *this file is spelled correctly and still incoherent*.

**Column 2 is `verify`'s too, and that is an obligation rather than a
convenience.** Because `verify` loads through `parse_for_inspection`,
which skips the loader's invariants so a broken map can still be
inspected, every rule the loader refuses on needs a counterpart here or
`verify` answers `valid` for a file the app will not open. The
zero-section rule had none: a node with `"sections": []` was reported
valid, exit 0, while the editor declined the file. That is checked
mechanically now —
`test_every_loader_invariant_has_a_verify_counterpart` reads
`check_invariants`'s own body and fails for a rule with no row.

`verify` sees all three. It loads through
`loader::parse_for_inspection` rather than the editor's strict door,
precisely so it can still open the files column 2 rejects — a tool
whose job is saying *what is wrong with this map* would be useless if
it could only read maps that were already fine. That path captures
unknown keys and skipped constructs exactly as the strict door does,
and drops only column 2's checks; without the capture, the two
reports in column 1 would be silently empty. So a violation listed in
column 2 appears in a `verify` report **and** blocks the load; one in
column 1 or 3 appears only in the report.

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
which silently wipes the theme — and since the render-time cascade
reads the palette first (see [palettes.md](./palettes.md)), a dangling
reference is the difference between a themed node and an unthemed one.
An empty palette produces no colors at any level.

`level` needs no rule of its own: it is a `usize` in the model, so a
negative value fails the load outright, and a value past the last group
is legal and clamps.

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

### Unknown keys

- Every key the loader kept without recognizing it, reported at the
  part of the document that carries it — `node "1.2"`, `edge[3]`,
  `palette "coral"`, `canvas`, `custom_mutations[0]`, or `map` —
  followed by the field path inside that part

**Why**: the load keeps such a key so a newer map is not damaged by an
older build, and warns about it once. Nothing else ever mentions it
again: it is written back at every save and read by nothing. A typo
(`min_zoom_to_rendr`) and a genuinely newer field look identical from
here, and both are worth a nonzero exit — one to fix, the other to
tell you the build is behind the file. See
[schema.md](./schema.md#unknown-keys-are-kept).

Interiors of `macros` and a node's `inline_macros` are exempt: they
are opaque JSON by design and nothing in them is ever unrecognized.

### Unknown variants

- Every construct the loader could not read at all and therefore
  skipped — a whole `custom_mutations[i]`, a node's
  `inline_mutations[i]`, or a node's or section's
  `trigger_bindings[i]` — reported at the part of the document that
  carried it, quoting serde's own account of what it refused
  (`unknown variant 'Glwo', expected one of ...`)

**Why**: this is the severe half of the same bargain as unknown keys,
and the reason it is a separate category is that the *consequence*
differs. An unrecognized key is inert — nothing reads it, so the map
behaves exactly as authored minus a key nobody acts on. An
unrecognized **variant** is the instruction: the load lifts the whole
construct out so a map from a newer build opens instead of showing
nothing, and what that construct described does not happen. The file
still has it and a save writes it back, but in *this* build the
mutation does not appear in `mutation list` and the trigger does not
fire. `{"mutator": {"Glwo": …}}` is a typo that silently costs
behavior, so there has to be a moment at which somebody finds out —
this is that moment, on a file that nonetheless loads and renders.

The unit is always the whole construct, never the part inside it that
failed: dropping one `Mutation` out of a macro would leave an entry
that still fires and silently does less than it says. Nothing else is
skippable — a node, an edge, the canvas or a palette this build cannot
read still fails the load outright. See
[schema.md](./schema.md#unknown-keys-are-kept).

## What's not checked

- **Color format** (`#RRGGBB` vs `rgb(...)` vs named colors): the format
  says hex or `var(--name)`, but the renderer is lenient. We don't verify
  color syntax — authors who type `"red"` will see default colors, and
  that's easy to diagnose visually.
- **Positions and sizes** are only *partly* unchecked now. A negative
  position is still valid — the canvas is unbounded — but a zero,
  negative, non-finite, or astronomically large extent is a
  load-blocking violation rather than a curiosity, because those
  numbers size real allocations downstream. See
  [Numeric domain](#numeric-domain).
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

## Numeric domain

Load-blocking. Every number a `.mindmap.json` carries into the scene
build has to be one the renderer survives, because the code
downstream of it is not defensive — the text shaper asserts, and the
border and connection builders size a `String` and a `Vec` from
authored geometry. `verify` reports these under the `numeric`
category; the loader refuses them outright.

| Rule | Bound |
|---|---|
| Font metrics — `font_size_pt`, `size_pt`, every min / max on borders, connections, labels, portals and text runs | finite, `0.5`..=`4096` pt |
| Min / max font pairs | not inverted (the cascades clamp with them) |
| Node extent | finite, positive, ≤ `1_000_000` per axis |
| Positions, section offsets, Bezier control points | finite, `|v|` ≤ `1e9` |
| Text runs | sorted, non-overlapping, non-empty, `end` ≤ the section's grapheme count |
| Connection body / cap glyphs | ≤ 16 grapheme clusters **and** ≤ 512 bytes (each is re-emitted per sampled point, and one cluster can carry unlimited combining marks — the cluster count keeps it a motif, the byte count keeps `bytes × samples` finite) |
| Border glyphs — all eight of `glyphs`, on every slot that carries a border config: a node's `style.border`, a section's `frame_border`, and the canvas's `default_border` / `default_section_frame_border` / `default_focused_section_frame_border` | ≤ 64 grapheme clusters **and** ≤ 1024 bytes. The four sides are patterns repeated to fill an edge; the four corners are emitted verbatim and looked up in the glyph-metric cache four times per node **per frame**. The shaping is memoized, but the cache key is an owned copy of the glyph built before the lookup, so an unbounded corner is an unbounded clone every frame and a permanent cache entry. Paired for the same reason as the connection glyph |
| Animation envelope — `duration_ms`, `delay_ms` | ≤ `60_000` ms each |
| Zoom windows | finite, non-negative, not inverted |
| Whole file | ≤ 256 MiB |
| Unrecognized keys per document | ≤ 100 000. Keys this build has no field for are kept and written back untouched, but each costs a heap-allocated capture route, and the file that buys one is about twelve bytes. Without a ceiling the byte limit above stops bounding memory: measured at roughly 575 bytes of peak RSS per captured key, a document at the 256 MiB limit reaches something on the order of 11 GB and the process dies before any check below it runs. Counted per *occurrence*, not per distinct name, so ten new keys on each of ten thousand nodes fits with room over |

The constants live in `lib/baumhard/src/mindmap/model/validate.rs`
and `lib/baumhard/src/font/fonts.rs` (with `MAX_NODE_AXIS` in
`model/node.rs` and the font window re-exported from `font/fonts.rs`),
and the loader, the document setters, and `verify` all read the same
ones.

The setters clamp rather than pass through, so a value the editor
writes is a value the loader accepts. That is a property maintained
by hand at each setter, not one the types enforce: the console
parsers are deliberately permissive — `parse_finite_pt` takes any
positive finite `f32`, the `spacing` verb any finite one — so a new
bounded field is a lockout waiting to happen until its setter is
clamped too. `border padding=` and `spacing` were exactly that, and
`test_extreme_editor_writes_still_reload` is where the property is
pinned: it drives each setter past its bound and round-trips through
the real save and the real strict load.

**Where a chokepoint cannot be arranged, make one.** `node.position`
is a public field with no setter, so there was nothing to guard: the
clamp went on one writer, then three, and a review round then found
six more — the interactive drag, four animation handlers, and the
custom-mutation sync-back, the last reachable from a map's own trigger
bindings rather than from the editor. Two of those six were missed
again by a hand-written enumeration and found only by the mechanical
scan.

So `MindNode::set_position_clamped` and `offset_position_clamped` are
now the only writers of a position *component*, and
`test_every_node_position_write_goes_through_the_clamp` reads the
workspace's own shipped source and fails the build for any other. The
offset entry point exists because the drag and animation writers
accumulate: clamping the sum is what bounds a coordinate that walks out
of the domain a step at a time, and clamping the delta would not.
Whole-struct assignment (`node.position = other`) is out of scope and
stays so — it copies a `Position` already in the domain rather than
deriving a new one.

Computed positions are **clamped**; authored ones are **rejected**, at
the loader and at `set_node_aabb` via `validate_node_position`. The
useful answer to a layout pass that ran off the canvas is the edge of
the canvas; the useful answer to a file that says `1e30` is no.

**Guard at the chokepoint, not at the caller.** Where several setters
write the same bounded field, the screen belongs in the one function
they share. The border glyphs are the cautionary case: the same
ceiling governs five slots across four setters, the screen was first
added to the per-node setter, and the other three kept writing maps
that would not reopen — while the per-node round-trip test passed and
made the gap invisible. `apply_glyph_border_edits_to_slot` holds it
now, beside the `font_size_pt` and `padding` clamps that were always
at that level and were always complete because of it.

`test_every_loader_bound_names_its_writer_side_guard` is the
registry that keeps the two ends in step. It **derives** the bound
set rather than listing it — every constant declared in baumhard that
`model/validate.rs` or `mindmap/loader.rs` consults in code — so a
bound added in any file, at any visibility, needs a row naming the
writer that guards it. What it cannot check is that the named writer
is the *only* one; that is what the per-surface round-trip cases are
for.

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
tree @ 0.0: parent_id "ghost" references a node that does not exist
ids @ 0.0: parent_id "ghost" does not match derived parent "0"
ids @ 1: HashMap key "1" does not match node.id "DIFFERENT"
references @ edge[0]: to_id "nowhere" is not a node
palettes @ 0: palette "nonexistent" is not defined in map.palettes
enums @ 0: style.shape "oblong" is not one of ["rectangle", "rounded_rectangle", "ellipse", "circle", "diamond", "parallelogram", "hexagon"]
numeric @ 0: zoom: min (2) is above max (0.5) — no zoom satisfies both bounds, so this never renders at any zoom level
numeric @ 0: section[0].text_runs[1] overlaps previous run (start 3 < previous end 5) — overlapping runs re-emit the same graphemes once per covering run
```

Each violation names its category, the location inside the file, and what
went wrong. The location format varies by category (node ID, edge index,
etc.) but is always clickable / greppable.
