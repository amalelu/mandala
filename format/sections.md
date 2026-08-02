# Sections

A **section** is a positioned text-bearing surface inside a
`MindNode`. Every renderable node carries at least one section;
a node's "user data strata" — the actual text the user types
into a node — lives on its sections rather than on the node
itself.

## Where they sit

```
MindMap
└── nodes[id]: MindNode
    ├── style, position, size, layout, channel, …  (node chrome)
    └── sections: [MindSection, MindSection, …]     ← user-data strata
        ├── text
        ├── text_runs
        ├── offset (relative to node.position)
        ├── size (None = fill the node)
        └── channel
```

In the runtime [Baumhard tree](../CONCEPTS.md#tree-t-m), a `MindNode`
materialises as a three-deep subtree:

- one **container** `GlyphArea` (chrome only — background, frame
  padding, shape, zoom window),
- one **section-area** `GlyphArea` per section, carrying the
  section's text and its theme-resolved `ColorFontRegions`,
- one structural **section-model** `GlyphModel` child per
  section-area, present as a future per-component / per-grapheme
  mutation seam (the renderer skips it today).

The renderer's tree walker shapes each section-area into its own
`cosmic_text::Buffer`, keyed by `unique_id`. No special-case in
the renderer — sections are first-class `GlyphArea` elements; the
multiplicity is the only thing the renderer notices.

## Field reference

| Field | Type | Default | Notes |
|---|---|---|---|
| `text` | string | required | The section's plain text. May contain `\n`. |
| `text_runs` | array | `[]` | Per-grapheme run table — see [text-runs.md](./text-runs.md). Empty means "render in the section/node defaults". Non-empty means "only the covered ranges render", same coverage trap as the pre-section single-runs vector. |
| `offset.x`, `offset.y` | number | `0.0` | Top-left of the section's AABB *relative to the owning node's `position`*, in canvas units. `(0, 0)` puts the section flush against the node's top-left. |
| `size` | object\|null | `null` | Section AABB. `null` means "fill the parent node" — the typical migration-default shape, where every node has one section that occupies its whole AABB. An explicit `{width, height}` lets a section occupy only part of the parent node, leaving room for siblings. |
| `channel` | integer\|null | `null` (falls through to section's index) | Mutation channel inside the parent node-area. `null` lets the tree builder substitute the section's index — a three-section node with no authored channels gets channels `[0, 1, 2]`. `Some(0)` is honoured even at idx > 0, so an author can deliberately collide with a sibling mind-node on channel 0 to broadcast. |
| `trigger_bindings` | array | `[]` | Per-section [`TriggerBinding`s](./mutations.md). The click dispatcher fires section-level bindings *before* the whole-node bindings on `MindNode.trigger_bindings` — a section-targeted override (e.g. a different `OnClick` mutation per stratum of a multi-section node) takes precedence over catch-all node bindings. |
| `frame_border` | object\|null | `null` | Per-section override of the cyan-rectangle frame drawn around the section in NodeEdit mode. Same shape as `MindNode.style.border` (preset / font / size / color / palette / glyphs / padding). `null` falls through to `Canvas.default_section_frame_border` (or `default_focused_section_frame_border` for the actively-edited section). The console verb `section frame …` authors this field; see [border-patterns.md](./border-patterns.md) for the full grammar. |

## Migration

Pre-section maps put `text` and `text_runs` directly on each
`MindNode`. The post-section data shape moves them into the
node's first section (and only section, in the default
migration). Per [`CODE_CONVENTIONS.md` §10](../CODE_CONVENTIONS.md)
"no dual shapes", the loader rejects pre-section files at parse
time with a concrete migration pointer:

```
legacy `text` / `text_runs` on node "0"; run
`maptool convert --sections <file>` to migrate node text into `sections[]`
```

`maptool convert --sections <in.json> <out.json>` walks every
node, lifts its legacy `text` + `text_runs` into a single default
`MindSection`, and writes the result back. The migration is
idempotent: re-running on an already-migrated map is a no-op.

The legacy `convert --legacy` pipeline (miMind import) folds the
section pass in automatically, so a single `convert --legacy`
hop produces a post-section file in one step.

## Custom mutations

Mutations authored as `CustomMutation` reach a node's section
text + runs at apply time. The flat-apply path
(`apply_custom_mutation` in
`src/application/document/custom.rs`) iterates every
`Flag::SectionRoot` child of each affected node container and
applies the mutation list to each — same primitive as
`apply_tree_highlights` walks. Mutations that target chrome
fields (`area.scale`, `area.position`) also land on the
container, so position-affecting mutations move the whole node
in lockstep with its sections.

Section content sync from the tree back to the model on
persistent mutations is wired: `sync_node_from_tree` walks every
section, writes back `text` / `text_runs` / `offset` / `size`
through the merge-with-prior reverse converter
(`region_to_text_run`). A **selective gate** keyed by per-region
`(range, color, font)` skips untouched sections so a
`SectionsOnly` text mutation doesn't silently strip
`bold` / `italic` / `underline` / `size_pt` / `hyperlink` from
sections the mutation didn't touch. Range-mutating mutations
(`ChangeRegionRange`, `SetRegionFont`/`SetRegionColor` over a
new range) inherit prior styling via dominant-overlap fallback
when no exact-range prior matches.

Documented round-trip limit: `var(--name)` colour references
collapse to their resolved hex on the round trip — the tree-side
`FloatRgba` carries no record of the variable. Authors who edit
section colours through custom mutations and then save the model
will see the variable replaced with a hex literal. The
`set_section_text` / `set_section_text_color` /
`set_section_font_size` / `set_section_font_family` document
setters bypass the round trip and preserve `var(--name)`
references verbatim.

### Subverbs (Batch 5 redesign)

The `section …` verb takes kv-style arguments per
SECTIONS_BORDERS_RESIZE_PLAN.md §4.5 — the prior positional
`<dx> <dy>` / `<w> <h>` forms are gone (CODE_CONVENTIONS §10:
no compatibility shims). Nine subverbs:

| Verb | Form | Effect |
|---|---|---|
| `section show` | `[section=<idx>]` | Multi-line resolved-property readout (text preview / runs / offset / size / channel / bindings / frame override). |
| `section move` | `dx=<f64> dy=<f64> [section=<idx>]` | Relative shift — `offset += (dx, dy)`. |
| `section move` | `x=<f64> y=<f64> [section=<idx>]` | Absolute set — `offset = (x, y)`. |
| `section resize` | `w=<f64> h=<f64> [section=<idx>]` | Pin `size = Some({w, h})`. |
| `section resize` | `fill [section=<idx>]` | Clear `size = None` (renamed from `none`). |
| `section text` | `"<text>" [section=<idx>] [runs=preserve\|clear]` | Replace text. `runs=preserve` (default) keeps per-grapheme styling on overlapping ranges; `runs=clear` collapses to single-run plaintext. |
| `section edit` | `[section=<idx>]` | Open the section text editor on the resolved target. Closes the console; modal handoff to the editor. |
| `section add` | `[at=<idx>] [text="<text>"]` | Insert a new section. `at=` defaults to append. |
| `section delete` | `[section=<idx>]` | Remove. Errors when only one section remains. |
| `section split` | `[section=<idx>] at=<grapheme>` | Split in two at the given grapheme boundary. `at=` is required (use `section show` to see the section's grapheme count); pass `at=N` where `N` equals the count to split at end-of-text (creates an empty suffix). |

Mutually exclusive forms reject at the parser layer:
`section move dx=1 x=2` errors with "cannot mix delta form
(dx/dy) and absolute form (x/y) — pick one" rather than
last-write-wins. Empty / unknown kvs surface usage hints.

Selection resolution: `Section` / `SectionRange` supplies the
index implicitly; `Single(node)` requires `section=<idx>`;
`MultiSection` rejects with a single-target hint for every
subverb **except** `section move dx=… dy=…`, which fans out
the same delta across every selected section atomically (Plan
§4.5 rule 4 — abort on first AABB rejection so a partial fan-
out never lands; absolute `x=`/`y=` form stays single-target
because identical absolute coords would collide). Macro / Action
path (`Action::SetSectionOffsetDelta`) is single-target on
MultiSection by design — script multiple Action steps for fan-
out.

Both verbs validate against the rules `maptool verify`'s
[`verify::sections`](./validation.md) enforces — finite +
non-negative offset, finite + strictly positive size, AABB
contained within the parent node's bounds, no astronomical
typos. Rejection messages are byte-equal to verify's so a
verb-rejected edit and a `verify` violation read identically.

### Effective size for AABB containment

`Some(sz)` honours the explicit pin; `None` (fill-parent —
the migration default) falls back to `node.size` for the
containment check. A `None`-sized section's effective AABB
is therefore `(offset, node.size)`, so any non-zero `offset`
makes the section stretch past the node's right / bottom
edge — verify flags it, both `set_section_offset` and
`set_section_size` reject it. Pre-fix the `None` arm
skipped the check entirely, leaving fill-parent sections
free to visually escape the parent (a degenerate state
authors could reach through `section move`, `section
resize fill`, or the drag gesture without any error
feedback).

The shared
[`MindSection::effective_size`](../lib/baumhard/src/mindmap/model/node.rs)
helper feeds the shared
[`model::validate`](../lib/baumhard/src/mindmap/model/validate.rs)
rules. Both the document-side setters and `maptool verify` route
through that validator, so the two cannot drift on what
"fill-parent" means or on the rejection messages.

**Drag-to-move gesture.** Click and drag on a section of a
multi-section node — past the drag-threshold the press promotes
to a section-only drag rather than a whole-node drag. The
section's `offset` updates per-frame in the tree; the model is
written once at release through `set_section_offset`, with the
same AABB validation as the verb (overflow snaps the section
back to its pre-drag offset and logs a message). Single-section
nodes still drag whole-node — mirrors the hit-test fold to
`HitTarget::NodeContainer`.

**Drag-to-resize gesture.** Selecting a `Some`-sized section
emits 8 resize handles on top of it — four corners plus four
edge midpoints (N / E / S / W). Click and drag a handle past
the drag-threshold to resize the section: corner handles move
both axes; edge-midpoint handles move only the perpendicular
axis. NW / N / NE / SW / W handles shift `offset` toward the
cursor while shrinking `size`, so the opposite edge stays put;
SE / E / S handles only grow `size` from the existing
top-left. Per-frame the section's tree position tracks the
cursor; the model is written at release through the atomic
`set_section_aabb` setter (one snapshot, one undo entry,
both `offset` and `size` validated together). AABB-overflow,
non-positive size, or astronomical-size rejection logs and
falls through to a model-side rebuild that snaps the section
back. `None`-sized (fill-parent) sections emit no handles —
their dimensions are owned by the parent's auto-fit floor, not
by an authored AABB.

After a position or size edit, the parent node's auto-fit
floor recomputes against **the larger of** measured text
bounds and (when set) user-pinned `size` plus `offset` — user
intent ("this section is at least this big") survives when
text fits, and text overflow still grows the parent so nothing
visually clips. Pre-Tier-2B the auto-fit pass skipped
`Some`-sized sections entirely; that gap is closed.

The auto-fit pass only **grows** the parent node's `size` (it
never shrinks). To explicitly shrink a node back to its
measured-text floor — useful after a manual `node resize`
that pinned the node larger than its content — use the
`node fit` console verb. The verb routes through
`MindMapDocument::fit_node_to_content`, which computes the
same floor the auto-fit pass uses (via the shared
`compute_one_node_text_floor` helper) and writes it
unconditionally as the new `node.size`, then re-applies
`grow_one_node_to_fit_border` so the rendered border has
room. Pinned-section contributions to the floor survive
verbatim — a `Some`-sized section's `(offset, size)` is part
of the measured floor, so `node fit` respects user intent
on every individual section just like the grow path does.

**Ambient auto-shrink is deliberately not part of the design.**
After "Hello World" → "Hi" the node stays at the longer
text's floor; the user reaches for `node fit` to recover
the slack. Asymmetric with the auto-grow side, but the
alternative — shrinking the node automatically on every
text-edit that frees space — fights with manually-set sizes
and produces janky resize-on-every-keystroke behavior.

**Section-side shrink path** is `section resize fill` (flatten
to fill-parent). A per-section `fit-to-content` is
intentionally absent — section text floor is folded into the
parent's `compute_one_node_text_floor`, so a `node fit` on a
multi-section node already reflows every section to its
content-driven floor.

Custom mutations (`target_scope: SectionsOnly` with
`AreaCommand::SetBounds` / `MoveTo` / `NudgeRight`) **bypass**
the AABB validation the verbs enforce — they write directly
through the tree-bridge sync path
(`document/custom/sync::sync_node_from_tree`). Authoring
through custom mutations can therefore produce out-of-AABB
sections that `maptool verify` rejects but the document
accepts. Run `maptool verify` after any mutation-driven
position/size authoring to catch the violations the verbs
would have refused.

### Console axis applicability on a section selection

Sections only have a **text** colour axis (`color text=…`). The
`bg=` and `border=` axes are node-level chrome and have no
section-level field — running them against a `SelectionState::Section`
returns `Outcome::NotApplicable` rather than collapsing to the
owning node's `background_color` / `frame_color`. This applies
both to the kv form (`color bg=#fff section=K`) and to the
trait-dispatch form (`color bg=#fff` with the section already
selected). The colour-picker modal follows the same rule:
opening the picker on `color bg` / `color border` against a
section selection surfaces the NotApplicable message rather
than opening the picker on the owning node's chrome axis.

The picker's read seed (`current_color_at` for a section handle)
and the write predicate (`set_section_text_color`) are
**cascade-symmetric**: if every run on the section shares one
colour that is the section's effective colour and is the
predicate the write rewrites against, otherwise both fall back
to `node.style.text_color`. So a uniformly customized section
opens the picker at its current colour and commits to a new
colour, instead of the picker silently no-op'ing because the
runs no longer match the node default.

`var(--name)` references on section runs survive the kv / trait
write paths verbatim (see "Documented round-trip limit" above).
The colour picker preserves them too, but only when the user
**doesn't move the wheel** from its open seed. Bit-exact equality
on `(hue_deg, sat, val)` is the "did the user touch it?" signal:
an open-and-close cycle with no interaction commits the original
`var(--accent)` literal back; any cell click or keyboard nudge
flips the commit to the new HSV's hex (the new colour was
explicitly chosen, so honouring the old reference would silently
discard it). Custom-mutation writes still collapse var refs to
hex on round-trip (`FloatRgba` carries no record of the variable);
that constraint is unchanged.

### Multi-section selection

Shift+click on a section extends the current selection
(`Section` ↔ `MultiSection`): each shift+click toggles the
targeted section in / out of the set. Cross-node section sets
are legal — `MultiSection(Vec<SectionSel>)` is dedup'd by
`(node_id, section_idx)`. Per-section verbs (`color text=…`,
`font size=…`, `font family=…`) fan out via the trait
dispatcher's `selection_targets` to apply to every section in
the set; `bg=` / `border=` continue to return `NotApplicable`
per the section spec (no chrome on sections). Per-section
gestures (drag-to-move, drag-to-resize) stay single-target —
a `MultiSection` selection emits no resize handles, and a
press on a section in the set **demotes** the selection down
to `Section(node, idx)` at threshold-cross so mid-drag picker
hints + per-section verbs reflect the in-flight gesture's
actual target rather than the prior multi-set. The
whole-node move and node-resize arms demote the same way (to
`Single(node)`). The `section move` / `section resize`
console verbs require a single-section context (or an
explicit `section=K` kv); `MultiSection` is fan-out-only at
the trait dispatch layer.

A click without shift on a section resets to the single-
section `Section(SectionSel)` shape; a click without shift on
a node resets to `Single(node_id)`. Building a multi-section
selection therefore always starts with a plain click, then
extends with shift+clicks.

### Section sub-range selection

A section can also be selected with a grapheme-level sub-range
via `SelectionState::SectionRange { sel, range: (start, end) }`.
The half-open `[start, end)` range targets a contiguous run of
graphemes inside one section's text; per-section verbs route to
range-aware setters (`set_section_text_color_range`,
`set_section_font_size_range`, `set_section_font_family_range`).

Two producers of `SectionRange`:

- **Console verbs** with `range=A..B` kv:
  `color text=#abc section=N range=2..7` /
  `font size=14 section=N range=2..7` /
  `font set <family> section=N range=2..7`. The verb path
  bypasses the trait dispatcher and calls the range setter
  directly; `range=A..B` requires `section=N` and rejects
  empty / inverted / non-numeric input with a clear error.
- **Editor shift-select anchor**: the inline text editor's
  `Shift+Arrow*` / `Shift+Home` / `Shift+End` keystrokes seed an
  anchor at the pre-action cursor position; subsequent shift-cursor
  moves extend the cursor while preserving the anchor. On editor
  close (commit or cancel), `lift_anchor_to_section_range` promotes
  a non-empty `(anchor, cursor)` pair to
  `SelectionState::SectionRange { range: (min, max) }`. Non-shift
  cursor moves and any text edit (typing / delete) clear the anchor
  — range-aware editing (typing-replaces-selection) is deferred.

**Clipboard contract.** `Cut` and `Paste` return
`Outcome::NotApplicable` rather than silently destroy
out-of-range graphemes; the action arm logs a `log::warn!`
line surfacing the skip. `Copy` falls through to whole-section
copy because it's non-destructive. Range-aware clipboard
semantics (sub-range cut, splice-paste) are deferred to a
future tier.

**Picker contract.** `color text` (no value) on a `SectionRange`
selection opens the picker with the sub-range plumbed through
`ColorTarget::Section { range }` → `PickerHandle::Section
{ range }`. The picker's `current_color_at` cascade scans
in-range runs only (via `text_run_ops::slice`); a range that
crosses a gap or contains disagreeing runs falls back to
`node.style.text_color`. Commit calls
`set_section_text_color_range` directly, bypassing the
`MultiSection` fan-out — different sections' lengths make
cross-section sub-range semantics incoherent. Section live
preview during wheel hover is deferred (commit-only today).

### Structured clipboard

Section copy / cut produce a structured payload carrying the
full per-section snapshot (`text_runs` + `offset` + `size` +
`channel` + `trigger_bindings`). The plain section text rides
the OS clipboard so cross-app paste sees a sensible string; the
structured payload rides an in-process buffer so a within-app
section→section paste round-trips per-run formatting and section
chrome.

The two halves stay coherent through a consistency check on
read: the structured payload is consulted only when the OS
clipboard's current text matches the buffer's snapshot exactly
(no trimming on either side, so a section whose text ends in
`\n` from the inline editor still round-trips). When the OS text
differs — typically because the user copied from another app
between Mandala copy and paste — paste falls through to a
plain-text branch where the new text takes the destination
section's first run as a formatting template (per-run structure
is lost; offset / size / channel / bindings stay).

Cut clears text and runs only; offset / size / channel /
bindings stay on the source section so the cut reads as "the
text disappeared" rather than "the section dissolved", and a
paste of the cut payload restores the full original shape.

## Channel space

Sections live in the same Baumhard tree as child mind-nodes. The
section channels and the child mind-node channels share one
sibling-channel space inside the container area. A custom
mutation that targets "channel 0 children" therefore hits both
the first section and any child mind-node tagged channel 0.

The seam closing this is the combination of
`TargetScope::SectionsOnly` and the
`GfxElementField::Flag(Flag::SectionRoot)` predicate variant —
both shipped. `SectionsOnly` walks every section directly via
`MindMapTree::section_arena_id` (bypassing the channel-keyed
sibling scan), and the predicate gate filters by element flag
within any other scope. See [mutations.md](./mutations.md) for
both authoring shapes.

The `MindSection.channel` field is `Option<usize>` (post Tier-E):
`None` falls through to the section's index, `Some(0)` is
honoured even at idx > 0. Pre-`Option`, the bare `usize`
silently overrode an author's explicit `channel: 0` on sections
beyond the first.

## Validation

`maptool verify` enforces:

- Per-section text-run invariants — non-overlapping, ascending,
  `end <= grapheme cluster count of section.text`. Same rules
  as the pre-section text_runs surface, just keyed by section.
  (Implemented in `crates/maptool/src/verify/text_runs.rs`.)
- Section offset / size shape: `offset.{x,y}` finite + non-
  negative; `size.{width,height}` (when set) finite + strictly
  positive; `offset + size <= node.size` (AABB containment).
- Node-level size sanity: `node.size.{width,height}` finite +
  strictly positive — a NaN at this level poisons every
  downstream AABB / hit-test / shaping computation.
- Astronomical section sizes: section `size > 100 × node.size`
  flags as a likely typo (e.g. accidental extra zero).
- Section channel collisions inside one node — surfaced because
  broadcasting one mutation across two sections sharing a
  channel is *occasionally* the intent but more often a typo.

The empty-`sections[]` invariant is enforced by the loader (not
verify): a zero-section map is rejected at parse time with a
`maptool convert --sections` migration pointer.

## See also

- [`schema.md`](./schema.md#mindsection) — the per-field type table.
- [`text-runs.md`](./text-runs.md) — per-grapheme styling, now
  anchored on a section instead of on the node.
- [`CONCEPTS.md` §3 "Sections"](../CONCEPTS.md) — conceptual
  treatment of the section selection model and per-section
  mutator reach.
