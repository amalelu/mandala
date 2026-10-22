# Migrating Legacy Maps

Earlier development iterations used a miMind-derived format with:

- Opaque numeric IDs (heap pointers like `"348068464"`)
- Integer enum codes (`"shape_type": 0`, `"anchor_from": 3`)
- Per-node color schemas duplicating the palette on every descendant
- `index: i32` for sibling ordering
- No `channel` field

Mandala no longer reads that format. A one-shot migration tool converts
legacy files to the current format.

## Also: migrating portals into edges

An earlier revision stored portals in a separate top-level
`portals[]` array. That parallel hierarchy has been folded into the
`edges[]` array — portals are now edges with
`display_mode = "portal"`. The current loader refuses to read a
file that still carries a `portals` key at all — an empty array is
still the old spelling, and the loader would otherwise keep it
forever as a key nothing acts on. This is the deliberate exception to
[keeping unknown keys](./schema.md#unknown-keys-are-kept): a key from
the *past* that the current model means something else by is not a
key from the future, and it has a one-command answer. Migrate with:

```
maptool convert --portals <input.json> <output.json>
```

Input and output may be the same path — see
[Writes are atomic](#writes-are-atomic). Each legacy `PortalPair`
becomes a `MindEdge` with `edge_type = "cross_link"`,
`display_mode = "portal"`, and the original glyph / color / font
carried into `glyph_connection`. The legacy `label` is dropped —
post-refactor portals identify by the edge tuple, and per-endpoint
label text lives on
[`portal_from` / `portal_to`](./portal-labels.md).

A legacy miMind map can carry portals too, so this pass also runs
automatically inside `convert --legacy` — right after the ID
rewrite, so the folded endpoints carry their new Dewey IDs. A
single legacy hop therefore produces a file the loader accepts;
there is no follow-up `convert --portals` to remember.

### The fold, exactly

Given this legacy entry:

```json
{
  "endpoint_a": "0.0",
  "endpoint_b": "0.1",
  "label": "Cross-reference",
  "glyph": "⬢",
  "color": "#ff00aa",
  "font": "LiberationSans",
  "font_size_pt": 18.0
}
```

the top-level `portals` key is removed and this edge is appended to
`edges[]`:

```json
{
  "from_id": "0.0",
  "to_id": "0.1",
  "type": "cross_link",
  "color": "#ff00aa",
  "width": 3,
  "line_style": "solid",
  "visible": true,
  "label": null,
  "anchor_from": "auto",
  "anchor_to": "auto",
  "control_points": [],
  "glyph_connection": {
    "body": "⬢",
    "font": "LiberationSans",
    "font_size_pt": 18.0
  },
  "display_mode": "portal"
}
```

Missing legacy fields take defaults: `glyph` falls back to `◈`
(an empty string counts as missing — a zero-width marker is
unclickable), `color` to `#aa88cc`, `font_size_pt` to `16.0`, and
`font` is omitted entirely when absent. A missing `endpoint_a` /
`endpoint_b` becomes an empty id rather than dropping the portal
silently — `maptool verify` then flags it as a dangling edge
reference, which is the diagnosable outcome.

Entries that are not JSON objects cannot become edges and are
**dropped**, each named on stderr with its index:

```
warning: portals[0] is a string, not an object; dropped
```

The reported count is the number of edges actually written, not the
length of the input array, so a summary line of `1 folded in from
legacy portals` next to three warnings tells you exactly what
survived. A `portals` key that is not an array at all is dropped the
same way, with the same kind of warning. (The loader rejects the
`portals` **key**, whatever its shape — an empty array and a string
are as unreadable to the current format as a populated one, and
keeping either would only mean carrying a dead spelling through every
future save.)

Both blocks above are read straight out of this file by
`convert::portals::tests::test_documented_fold_matches_converter_output`,
which parses them and compares against what the converter emits — so
editing either block fails that test rather than silently drifting
away from the code.

## Also: migrating node text into sections

The post-section data shape moves a node's `text` and `text_runs`
fields onto the node's first
[`MindSection`](./sections.md). The current loader refuses
to read a file that still carries node-level `text` / `text_runs`;
migrate with:

```
maptool convert --sections <input.json> <output.json>
```

Each legacy node becomes a node with one default
`MindSection { text, text_runs }` and no other surface change. The
migration is idempotent (re-running on an already-converted map is
a no-op) and runs automatically inside `convert --legacy` for
miMind imports, so a single legacy hop produces a post-section
file in one step.

## The legacy-format command

```
maptool convert --legacy <input.json> <output.json>
```

Reads `<input.json>` as a legacy-format file and writes `<output.json>` in
the current format. Unless the two paths are the same, the input is
never modified.

## Writes are atomic

All three `convert` verbs write the same way: the finished JSON goes
to a staging file (`<dir>/.<name>.<pid>.tmp`) which is then renamed
over the output path. The existing file at that path is never opened
for writing, so an interrupted run — a kill, a full disk, a crash —
leaves either the original file intact or the converted file
complete, never a truncated partial. That is what makes passing the
same path for input and output safe on every verb, not just
`--portals`. The app's own save path (`save_to_file`) uses the same
writer.

**The output is a new inode.** That is the mechanism, not a detail:

- The file's **permissions are preserved** when the output path
  already exists — a map you `chmod 600` because it carries private
  notes stays owner-only across an in-place convert, rather than
  reverting to the umask default.
- **Hard links are detached.** Another name for the old file keeps
  the *old* content; it does not follow the conversion.
- **A symlink at the output path is replaced**, not followed: after
  the convert, that path is a regular file holding the converted map,
  and the link's former target is untouched.

If you rely on either of the last two, convert to a distinct output
path and move the result into place yourself.

## What it does

1. **Assigns Dewey-decimal IDs** by walking the tree (using `parent_id` +
   the old `index` field for sibling order). Rewrites every reference —
   edge `from_id`/`to_id` (covers both line-mode and portal-mode edges;
   post-refactor portals live in the edges array), legacy
   `portals[].endpoint_a`/`endpoint_b`, and the HashMap keys.
2. **Folds legacy `portals[]` into portal-mode edges** and removes the
   top-level array — the same pass `convert --portals` runs, applied
   here right after the ID rewrite so the folded endpoints carry
   their new Dewey IDs. See
   [The fold, exactly](#the-fold-exactly) above.
3. **Converts integer enums to named strings** for `shape_type` →
   `shape`, `layout.type`, `layout.direction`, `line_style`,
   `anchor_from`, `anchor_to`. Unknown integer values fall back to
   sensible defaults (documented in each enum's value list —
   see [enums.md](./enums.md)).
4. **Hoists color schemas to top-level palettes**. Each unique palette is
   defined once; per-node `color_schema` becomes a lightweight reference.
   The `theme_id` and `variant` fields are dropped; `variant` != 2 gets
   folded into the palette name (`"coral"` + `variant: 3` becomes
   `"coral-v3"`). A negative `level` is floored to 0 with a message —
   the model types it `usize`, so emitting one would produce a file
   this tool could not read back.

   **The hoisted palette is authoritative at render time.** The
   converter leaves each node's `style` colors where it found them,
   but they are now the *fallback*: a node carrying a `color_schema`
   draws its fill, frame and text from the palette group
   (see [palettes.md](./palettes.md)), and its `style` colors are what
   it would use if the schema were removed. A converted map whose
   `style` values were hand-edited away from the palette after
   conversion will therefore render from the palette, not from the
   edits. Three fixes, most specific first: move the edit into that
   node's `color_schema.overrides`, which excepts one channel on one
   node and is where `color bg=…`, the glyph-wheel picker and every
   other interactive setter already write; edit the palette, when
   what drifted was the theme rather than the one node; or drop the
   `color_schema` from the nodes that were deliberately detached,
   which returns them to their `style` colors wholesale.
5. **Removes `index`** from each node (sibling order derives from the new
   Dewey ID).
6. **Adds `channel: 0`** to each node (the default).
7. **Folds node `text` / `text_runs` into `sections[0]`** — the same
   pass `convert --sections` runs, applied last so an already-cleaned
   tree converges on the post-section shape.

### miMind `shape_type` ordinals

Step 3 rewrites the integer `style.shape_type` as the named
`style.shape`. The mapping is miMind's own Java enum order:

| `shape_type` | `shape` |
| --- | --- |
| 0 | `rectangle` |
| 1 | `rounded_rectangle` |
| 2 | `ellipse` |
| 3 | `diamond` |
| 4 | `parallelogram` |
| 5 | `hexagon` |

Any other value — including a non-integer — converts to `rectangle`,
ordinal 0, which is also the format default and the shape the runtime
substitutes for anything it cannot draw.

**This table is the contract, not a description of the code.**
`LEGACY_SHAPE_ORDINALS` in `crates/maptool/src/convert/enums.rs` is an
array indexed by the ordinal, so a row inserted in the middle would
silently renumber every row below it and quietly change what old files
convert to. `legacy_shape_ordinals_match_the_published_table` reads
this table back off disk and fails when the two disagree — on row
count, on any row's spelling, and on the numbering itself, which has
to be each ordinal `0..n-1` exactly once, so a duplicated ordinal
cannot quietly retire the row it displaced.
`legacy_shape_ordinals_are_canonical_spellings` separately holds every
spelling here to `KNOWN_SHAPES` in
`lib/baumhard/src/gfx_structs/shape.rs` — the one list the runtime
classifier and `maptool verify` both consult, so a spelling that
drifted out of it would make `convert --legacy` write files
`maptool verify` rejects. This table is restatement 5 of the five
[enums.md](./enums.md#styleshape) counts; it is pinned to
`LEGACY_SHAPE_ORDINALS`, which is pinned to `KNOWN_SHAPES`.

The other five enums step 3 converts (`layout.type`,
`layout.direction`, `line_style`, `anchor_from`, `anchor_to`) have no
published ordinal table and no pin; their mappings live only in
`convert/enums.rs`. Giving them the same treatment is a separate
change.

## Known limitations

- **Orphaned nodes** (nodes whose `parent_id` references a non-existent
  node) keep their original ID — they can't be placed in the Dewey tree
  without a parent. The output is internally consistent but has mixed ID
  styles. Fix the input or edit the output.
- **Unknown enum values** fall silently to defaults. If you had a custom
  shape code that meant something specific, it becomes `"rectangle"`.
- **Palette collisions** (two level-0 nodes with the same palette name +
  variant but different `groups`): first-writer-wins. Rare in practice
  because miMind produces consistent palettes across nodes in the same
  theme.

## After conversion

Run `maptool verify <output.json>` to confirm the converted file is
well-formed. It should exit 0 with no violations. If it doesn't,
the input had structural problems the converter couldn't resolve (cycles,
orphaned nodes, etc.).

## Why a separate tool?

Mandala rejects legacy files at load time rather than silently migrating
them. The format drift is too large to patch over with `#[serde(alias)]`
and backward-compat struct fields — that approach bakes the legacy format
into the runtime indefinitely. A dedicated migration tool keeps the
runtime clean: it only ever reads the current format.

That is not in tension with the loader keeping keys it does not
recognize. The two rules answer different questions. An unrecognized
key means *this build is older than the file* — there is nothing to
migrate and nothing to decide, so the key rides along untouched. A
legacy key means *the file is older than the format* — the name is
taken, the migration exists, and preserving it would only carry a
contradiction forward.

The conversion is idempotent-safe for files that already look current
(already-Dewey IDs survive unchanged, already-string enums pass through,
already-hoisted palettes don't double-hoist). But the converter is
intended as a one-shot migration, not an always-on pipeline.
