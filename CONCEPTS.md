# Mandala & Baumhard — Conceptual Building Blocks

*A reference for the named concepts that make up this project.*

---

## On this document

Mandala is a mindmap application; Baumhard is the glyph-animation
library it is built on. They are one project
([`CODE_CONVENTIONS.md §1`](./CODE_CONVENTIONS.md)). Together they
have accumulated a vocabulary — `GlyphArea`, `MutatorTree`, `Channel`,
`Portal`, `ZoomVisibility`, `ThrottledInteraction`, `CustomMutation`,
and so on — that sits deliberately across the thin line between *user*
and *developer*. The project aims to expose as much power to end
users as the architecture will carry, so even a curious non-programmer
benefits from knowing what the pieces are and how they fit.

This document names every load-bearing concept, says what problem it
solves, and shows where to reach for it. It is **not** a tutorial,
**not** a schema spec (see [`format/`](./format/) for that), and
**not** a set of prescriptions (see
[`CODE_CONVENTIONS.md`](./CODE_CONVENTIONS.md) and
[`lib/baumhard/CONVENTIONS.md`](./lib/baumhard/CONVENTIONS.md) for
those). It is a *reference*: one place to ctrl-F when a term is
unfamiliar, one place to browse when getting oriented, one place to
point a new contributor at.

The codebase is young and its ambitions are wide. Much of what is
here is a foundation for more. Where a concept has a seam that is
wider than strictly needed today, that is usually because a *named
trajectory* is expected to attach there later — plugins, a Baumhard
script API, richer animations, complex file exports. The "extra
ceiling height" is the point, not the accident
([`CODE_CONVENTIONS.md §7`](./CODE_CONVENTIONS.md)). Entries flag
these seams explicitly.

Each entry uses bold labels in this order: **Summary** (one
sentence), **What it's for** (the problem it solves), **Under
the hood** (file references in `path/to/file.rs:line` form, jump
targets), and where useful **Vision** (named trajectory) and
**Caveat** (gotchas).

## Table of contents

- [§1 Project foundations](#1-project-foundations)
- [§2 The Baumhard foundation](#2-the-baumhard-foundation)
- [§3 The mindmap domain](#3-the-mindmap-domain)
- [§4 The mutation framework](#4-the-mutation-framework)
- [§5 The application runtime](#5-the-application-runtime)
- [§6 The authoring surface](#6-the-authoring-surface)
---

## §1 Project foundations

Eight cross-cutting stances shape almost every concept below. None of
them are invented here — the canonical statements live in
[`CODE_CONVENTIONS.md`](./CODE_CONVENTIONS.md) and
[`lib/baumhard/CONVENTIONS.md`](./lib/baumhard/CONVENTIONS.md) — but
they are named here so the rest of this document makes sense without
detour.

### Mandala and Baumhard are one project

Baumhard is not a dependency we use; it is a foundation we build.
Both crates are ours. When a feature needs a primitive Baumhard does
not yet have, the primitive is added to Baumhard rather than worked
around in the app. See [`CODE_CONVENTIONS.md §1`](./CODE_CONVENTIONS.md).

### Mutation-first

Any data-model change is a **mutator** applied to a **tree**, never
clone-edit-reinsert. See
[`lib/baumhard/CONVENTIONS.md §B2`](./lib/baumhard/CONVENTIONS.md).

### Everything is glyphs

Text, borders, connection lines, portal markers, console chrome,
selection highlights — every visual element is a positioned font
glyph. There are no rectangle-shader UIs, no bitmap sprites, no
icon atlases. Introducing a new visual is a question of "what glyph
goes where", not "add a new pipeline".

### Single-threaded event loop

`Application` owns the `Renderer` directly. No channels, no worker
threads, no `tokio`, no `std::thread::spawn` in any interactive
path. The one sanctioned exception running today is the native
[`FreezeWatchdog`](#freezewatchdog) thread, which only *reads* an
`AtomicU64` ping; CODE_CONVENTIONS §3 additionally sanctions the
[IPC boundary threads](#ipc-server-ipcserver), which run only under
`--ipc`, move protocol values and never touch app state. Lock
scopes stay trivial because of this.

### Model / view separation

The [`MindMapDocument`](#mindmapdocument) owns the data; the
[`Renderer`](#renderer) owns GPU resources. The renderer reads
intermediate representations from the document each frame; it
never reaches into the document. The document never holds GPU
handles.

### Cross-platform as first-class

Native desktop and the browser are equal deployments. Full
prescriptive rules: [`CODE_CONVENTIONS.md §4`](./CODE_CONVENTIONS.md).
Live parity status: [`CLAUDE.md`](./CLAUDE.md) "Dual-target status".

### Canonical or exemplary

The bar for every merged change is *canonical* or *exemplary*. See
[`CODE_CONVENTIONS.md §0`](./CODE_CONVENTIONS.md). "Not caused by
my changes" is not an excuse — if you notice a gap, you own the
close.

### Preserved seams

A **seam** is a point where a future extension can attach without
rewriting what surrounds it. Seams are named throughout this
document; "extra ceiling height" is deliberate. See
[`CODE_CONVENTIONS.md §7`](./CODE_CONVENTIONS.md).

---

## §2 The Baumhard foundation

Baumhard is the glyph-animation library under
[`lib/baumhard/`](./lib/baumhard/). It is where most of the
conceptual vocabulary of the project originates. The mindmap layer
(§3) and the application layer (§5) reach into it constantly; most
of their own concepts are compositions of the primitives below.

For the prescriptive rules of the crate — the mutation-first
discipline, the arena invariants, the unsafe policy, the benchmark
obligations — see
[`lib/baumhard/CONVENTIONS.md`](./lib/baumhard/CONVENTIONS.md).
This section is conceptual.

### `Tree<T, M>`

An arena-backed forest of typed nodes with cached
spatial indices, representing one layer of visual content.

A `Tree` is how Baumhard stores anything
hierarchical that needs to render or be hit-tested: one tree for the
mindmap nodes, one for connection glyphs, one for borders, one for
the console overlay, and so on. Every node in a tree is reached
through an opaque `NodeId` — not a pointer, not an index — so the
tree can be rearranged, cached, or serialized without invalidating
references. Nodes are `Clone`, mutation is in-place (never
rebuild-the-arena), and both AABB caches and an optional region
index ride along so hit-testing stays cheap.

Defined in
`lib/baumhard/src/gfx_structs/tree.rs`. Wraps `indextree::Arena<T>`;
adds `root: NodeId`, `layer: usize`, an AABB cache (`Cell<Option<...>>`
because the values are `Copy`), a subtree-AABB dirty flag, and an
optional `RegionParams` + `RegionIndexer` seam for future spatial
queries. The blessed iteration primitives are
`NodeId::children(&arena)` and `descendants(&arena)`; collecting
into a `Vec<NodeId>` is a code smell. Every `MutatorTree::apply_to`
call invalidates the AABB cache once, not per-field.

### `MutatorTree<M>`

The mutation-side mirror of a `Tree`: same shape,
carrying deltas instead of values.

If `Tree` is the *noun*, `MutatorTree` is the
*verb*. A `MutatorTree<GfxMutator>` describes a change to apply to a
`Tree<GfxElement, GfxMutator>`: "mutate the third child's text,
shrink the font on every descendant of channel 2, repeat until the
predicate fails." The tree walker pairs the two up by channel (or
sibling position, depending on the instruction), applies matching
deltas in place, and leaves the rest alone. This is the seam custom
mutations ([§4](#4-the-mutation-framework)) ride on.

Also in
`lib/baumhard/src/gfx_structs/tree.rs`. Minimal — an `Arena<T>` and
a `root: NodeId`. No spatial data: mutators are pure deltas, they do
not render. The trait bound `TreeNode` requires a `void()` sentinel
for padding the mutator's shape to match the target's when channels
do not line up. `MutatorTree<GfxMutator>::apply_to(&mut target)` is
the whole entry point; it calls `walk_tree_from` under the hood.

### `Applicable<T>`

A one-method dispatch trait: "apply this delta to that
value".

Almost every mutation primitive in Baumhard
implements `Applicable` against its target type. `MutatorTree<M>
: Applicable<Tree<T, M>>` is the big one, but there are also
`DeltaGlyphArea: Applicable<GlyphArea>`,
`DeltaGlyphModel: Applicable<GlyphModel>`,
`GlyphAreaCommand: Applicable<GlyphArea>`, and so on. The shape is
always `fn apply_to(&self, target: &mut T)`. This keeps the
vocabulary uniform: to learn how a new delta works, look at its
`apply_to` and nothing else.

Defined in `lib/baumhard/src/core/primitives.rs`.
The trait is deliberately minimal; no associated types, no `Result`.
Interactive paths cannot panic
([`CODE_CONVENTIONS.md §9`](./CODE_CONVENTIONS.md)), so type
mismatches (e.g. applying a `ModelDelta` to a `GlyphArea` target)
are silently ignored by design — the dispatch site is responsible
for well-typed pairing. This tradeoff is ugly but correct for a
real-time editor: the cost of a dropped mutation is a visual
glitch on one frame; the cost of a panic is a lost document.

### `ApplyOperation`

The operation selector a delta carries — `Add`,
`Assign`, `Subtract`, `Multiply`, `Delete`, or `Noop`.

A single `DeltaGlyphArea` does not hardcode
"text replace" vs. "text append"; it carries an `ApplyOperation`
that tells the generic `apply` helper which trait assignment to
use. That is how one `Text(String)` delta variant covers both
"concatenate this suffix" (`Add`) and "replace the whole text"
(`Assign`) without duplicating the variant.

`lib/baumhard/src/core/primitives.rs`. The
generic apply requires the target type implement `AddAssign`,
`SubAssign`, `MulAssign`, and `Default` — which is why every
mutable field type in the delta world carries all four.

### `GfxElement`

The tagged union every `Tree<GfxElement, _>` node is
— either a `GlyphArea`, a `GlyphModel`, or a `Void`.

This is *the* tree-node type in the codebase.
All visual things — every text region, every composed-glyph
shape, every structural padding node — is one variant of this
enum. Shared metadata rides on every variant: a `channel` for
mutation routing, a `unique_id` assigned by the host app (Mandala
uses it for the mindmap node id), a `flags` set, an
`event_subscribers` list, and a cached `subtree_aabb`.

`lib/baumhard/src/gfx_structs/element.rs`.
`GlyphArea` and `GlyphModel` each box their payload (one heap
allocation per element of that kind); `Void` has no payload.
There is a companion `GfxElementType` enum for cheap variant
checks without destructuring, and a `GfxElementField` enum used
by predicates and field-level mutations to name "which part of
which variant". `GfxElementType` — like every other `*Type` tag in
the pipeline (`MutatorType`, `MutationType`, `GlyphAreaFieldType`,
`GlyphModelFieldType`, `GlyphAreaCommandType`,
`GlyphModelCommandType`) — is *derived* from its enum via strum's
`EnumDiscriminants`, so a tag can never drift from the variants it
names. All of them are read through one trait,
`core::primitives::Discriminated`, which supplies `variant()` and
`same_type()`.

### `GlyphArea`

A text region — the only element that actually draws
glyphs to the screen.

When something visible has characters in it, a
`GlyphArea` represents it: mindmap node text, connection glyphs,
portal icons, console lines, FPS overlay digits. The struct
carries everything the renderer needs to shape and draw that text
— position, render bounds, font scale and line-height, per-span
color/font overrides ([`ColorFontRegions`](#colorfontregions)), a
background fill, an optional outline halo, a hit shape, and a
zoom-visibility window.

`lib/baumhard/src/gfx_structs/area.rs`. Uses
`OrderedFloat<f32>` and `OrderedVec2` for its numeric fields so
the struct is `Eq + Hash` despite holding floats — important for
caching and identity-based diffing. The `hitbox` field is the one
exception to the hash/eq contract: it is derived by the scene
builder from the rest of the fields, not part of identity. One
`GlyphArea` maps to one cosmic-text `TextArea` in the renderer.

The `text: String` is edited with grapheme-aware
helpers ([`grapheme_chad`](#utilities--grapheme_chad-color-geometry)); byte
offsets from user-facing counts will land mid-cluster on the
first emoji.

### `GlyphModel`, `GlyphMatrix`, `GlyphLine`, `GlyphComponent`

A four-level composition hierarchy for glyph shapes
built out of small typed cells.

Sometimes a visual element is more structured
than a plain string — a grid, a menu, a composed diagram built of
box-drawing pieces. `GlyphModel` is the answer: it is a child of a
`GlyphArea` that contributes a matrix of lines of components, each
component carrying its own text plus optional font and color
overrides. The model paints its contents *into* the owning
`GlyphArea`'s buffer at shape time, so the whole thing shapes and
renders as one cosmic-text pass while remaining structurally
addressable for mutation.

The hierarchy is: `GlyphModel` owns a
`GlyphMatrix`; `GlyphMatrix` owns a `Vec<GlyphLine>`; `GlyphLine`
owns a `Vec<GlyphComponent>`; `GlyphComponent` is
`{ text, font: Option<AppFont>, color: Option<FloatRgba> }`. All
files in `lib/baumhard/src/gfx_structs/model/`. Matrix/Line both
auto-expand on out-of-range index write, so callers can poke at
arbitrary coordinates without pre-sizing. The central
`GlyphMatrix::place_in` method paints the matrix into the owning
area's `String + ColorFontRegions`, padding with newlines and
spaces so every component lands on the intended grapheme cell.

### `Void`

A no-op tree node: no payload, no render cost, just
structure.

Sometimes a mutator tree needs a child at index
*k* that does nothing, so subsequent children align against the
right target children. Sometimes a target tree needs a parent that
has no content of its own but holds other elements. `Void` is the
answer in both cases. It is never required — but used tastefully,
it keeps tree shapes regular and channel alignment clean.

`lib/baumhard/src/gfx_structs/element.rs` for
the target side; same enum on the mutator side in `mutator.rs`.
No heap allocation, just metadata (channel, id, flags).

### `ColorFontRegions`

A set of character-range spans, each with optional
color and font overrides, layered over a `GlyphArea`'s text.

A single node's text can have multiple styles —
a bold first word, a red annotation, a smaller footnote. Rather
than fragmenting text into per-style nodes, Baumhard carries
**span tables**: `[start, end)` ranges that say "between these
two positions, use this color and/or this font". Any part of the
text not covered by a span inherits the area-level defaults.
The same primitive drives rich-text on mindmap nodes
([`text runs`](#text-runs)), highlight on selected regions, and
transient live-edit previews.

`lib/baumhard/src/core/primitives.rs`. Backed
by `BTreeSet<ColorFontRegion>` keyed on the `Range`, so lookups
by range are `O(log n)` but two regions with the same range and
different payloads collide (last write wins) — this is
deliberate, not a bug. The `Range` indices are **grapheme-cluster
offsets** — the unit baumhard's text primitives speak in (see
`lib/baumhard/CONVENTIONS.md §B1` and the helpers in
`util/grapheme_chad.rs`). Every fresh producer counts via
`count_grapheme_clusters`; the cosmic-text bridges in
`font/attrs.rs` slice through `find_byte_index_of_grapheme`. The
primitive itself just holds `usize` pairs and does not enforce a
unit at the type level, so consumers that reach in from elsewhere
must agree on the grapheme convention. Five mutation primitives keep the set
consistent under text edit: `insert_regions_at`,
`shrink_regions_after`, `split_and_separate`,
`shift_regions_after`, `set_or_insert`. A spatial index
([`RegionIndexer`](#regionparams-regionindexer-regionerror)) can
be layered on top for hit-testing.

Never mutate `ColorFontRegions` outside the mutator
pipeline — direct writes skip the index update and selection
drifts silently. See [`lib/baumhard/CONVENTIONS.md §B6`](./lib/baumhard/CONVENTIONS.md).

### `Range`

A half-open `[start, end)` span of `usize` indices.

The canonical primitive for "some part of the
text" everywhere text appears. `ColorFontRegion` keys on it;
`GlyphAreaCommand::ChangeRegionRange` manipulates one; text-run
schema validation runs over them. Small but load-bearing: a
single shared `Range` type means span operations compose across
modules without glue.

`lib/baumhard/src/core/primitives.rs`. Totally
ordered for `BTreeSet` use; ships with `magnitude`, `push_left`,
`push_right`, `overlaps`, `to_rust_range`.

### `Channel` and `BranchChannel`

An integer routing tag on every node. The tree walker
matches mutator nodes to target nodes by equal channel within a
sibling group.

Without channels, every mutation applied to a
parent would broadcast to *every* child; with channels, the author
can say "this mutation only hits siblings tagged channel 1". A
parent and its child can share a channel or differ; the matching
is within-sibling only. Siblings on the same channel form a
*broadcast group*: one mutator affects all of them. This is the
primitive that makes a single mutation selective without naming
child indices.

The `BranchChannel` trait
(`lib/baumhard/src/gfx_structs/tree.rs`) is a one-method trait
`fn channel(&self) -> usize`. Both `GfxElement` and `GfxMutator`
implement it. The walker calls it to align children. In the
mindmap domain, `MindNode.channel` is where this surfaces to
end users; see [§3: Channels](#channels-mindmap-level).

Children arrive at the walker in **Dewey-id order**
(`id_sort_key`), not in channel order — the tree builder sorts
by id, not by channel. Channel matching happens within whatever
sibling order the map defines. Authoring custom mutations that
target specific channels therefore means arranging children so
that channel order and id order agree, or reaching for the
[`MapChildren`](#instruction) instruction to pair strictly by
sibling position instead.

### `Flag` / `Flaggable` / `AnchorBox`

A small enum of state markers any node can carry, and
the trait that queries them.

Some per-node state is not *data* in the
rendering sense but *status* — "this node is focused", "this
node is in edit mode", "this node is anchored to a specific
screen corner". Flags provide a uniform place to store those,
queryable by [predicates](#predicate-and-comparator) without
extending the element's data fields.

`lib/baumhard/src/core/primitives.rs`. Current
variants: `Focused`, `Mutable`, `Anchored(AnchorBox)`,
`MutationEvents`, `SectionRoot`. `AnchorBox` holds up to four
`Anchor` entries for layout-solver pinning. `MutationEvents` is
reserved — it marks a node that should fire events on mutation (a
seam for future reactive handlers).

`Anchored` and its `AnchorBox` / `Anchor` / `AnchorPoint` /
`AnchorTarget` payload have no layout solver behind them yet, and
nothing in either crate constructs one. They survive the #41
dead-code sweep on format grounds rather than code grounds:
`Flag` is `Serialize` / `Deserialize` and reachable from a
`.mindmap.json` through the `GfxElementField::Flag(Flag)`
predicate language, where
[`format/mutations.md`](./format/mutations.md) publishes
`Anchored(AnchorBox)` as an authorable variant. Removing the
variant would change what an authored map round-trips, which is a
format decision and not a cleanup.

### `Event`, `GlyphTreeEvent`, `GlyphTreeEventInstance`, `EventSubscriber`

A *non-state-mutating* kind of mutator: instead of
changing element data, it invokes callbacks subscribed to the
element.

Event-driven behavior (button-like nodes,
hover-response, keyboard dispatch to a focused node) does not
belong in the mutation-first data pipeline — a keystroke is not
a delta to a field. Events reuse the mutator infrastructure for
dispatch but invoke subscriber callbacks instead of editing
data. A subscriber *can* enqueue further mutations as a
reaction, which is how reactive chains are built.

`lib/baumhard/src/gfx_structs/mutator.rs`.
`GlyphTreeEvent` is the enum of event kinds (`KeyboardEvent`,
`MouseEvent`, `AppEvent`, `CloseEvent`, `KillEvent`);
`GlyphTreeEventInstance` wraps it with a timestamp;
`EventSubscriber` is
`Arc<Mutex<dyn FnMut(&mut GfxElement, GlyphTreeEventInstance)
 + Send + Sync>>`. The `Arc<Mutex<…>>` shape exists so that
cloning an element (as the arena does) keeps a single callback
reachable from every clone rather than duplicating state.

Today the mindmap app does not use subscribers
heavily — most interaction goes through the application's own
input handlers. The seam is preserved for the Baumhard script
API and plugin trajectory, where user-authored code will want
to subscribe to events without reaching into the app crate.

### `Predicate` and `Comparator`

A small expression language for "does this element
match?" tests, used by loop and dispatch instructions.

Some mutations only apply to certain nodes —
"every child whose font size is under 12pt", "every descendant
marked `Focused`". A `Predicate` names the fields to test and
the `Comparator` (equals, not-equals, greater-than, etc.) to use
against each; the walker evaluates it per candidate node and
decides whether to recurse.

`lib/baumhard/src/gfx_structs/predicate.rs`.
Pure data (serializable); typical predicates carry one or two
fields, so evaluation is effectively `O(1)`. Float comparisons
use `almost_equal` with a `1e-5` epsilon
([`util/geometry.rs`](#utilities--grapheme_chad-color-geometry)). The
`Comparator` uses a *negation flag* pattern: `Equals(false)` is
`==`, `Equals(true)` is `!=`, halving the variant count.

### `Instruction`

The four control-flow primitives a `GfxMutator` can
carry: `RepeatWhile`, `SpatialDescend`, `MapChildren`, and
`RotateWhile` (reserved).

Most mutations are direct: apply this delta to
this node. Some need to loop ("apply this to every descendant
matching predicate X"), some need spatial routing ("apply this
to whichever node contains this point"), some need
position-indexed pairing ("apply these N mutators to these N
siblings, zip-style"). `Instruction` is the vocabulary. This is
how one custom mutation can sweep a whole subtree without
hand-listing every target.

`lib/baumhard/src/gfx_structs/mutator.rs`.
- `RepeatWhile(Predicate)` — iterates children, applies mutator
  children while predicate holds, stops on failure. Aligns by
  channel (broadcast semantics).
- `SpatialDescend(OrderedVec2)` — finds the deepest node whose
  subtree AABB contains the given point, applies the mutation
  there. Bypasses channel alignment.
- `MapChildren` — zips mutator children to target children
  **strictly by sibling position**, ignoring channels. The
  right shape for size-aware layouts where index matters more
  than tag.
- `RotateWhile(f32, Predicate)` — reserved AST variant; walker
  is a no-op stub today.

### `GfxMutator`

The mutator-side node type, mirroring `GfxElement`:
`Single`, `Macro`, `Void`, or `Instruction` variants.

Every node of a `MutatorTree` is a
`GfxMutator`. The four variants cover "one field change here",
"a batch of changes on this target", "structural padding", and
"control flow with nested children". Together with
[`Instruction`](#instruction) and
[`Predicate`](#predicate-and-comparator) they form a small but
complete mutation language.

`lib/baumhard/src/gfx_structs/mutator.rs`.
Implements `BranchChannel`. The `Mutation` payload can be an
`AreaDelta`, `AreaCommand`, `ModelDelta`, `ModelCommand`,
`Event`, or `None`. A `Macro` carries a `Vec<Mutation>` applied
in order to the same target, plus optional `children` for
descendant instruction nodes.

### `Mutation` enum

The payload union: which kind of delta or command
this mutator carries.

A mutation is not one uniform thing — a
`GlyphArea` and a `GlyphModel` accept different kinds of change.
The `Mutation` enum is the sum type covering all of them: two
flavors each for area and model (field-level `Delta` vs.
imperative `Command`), plus `Event` (subscriber dispatch) and
`None` (structural placeholder).

`lib/baumhard/src/gfx_structs/mutator.rs`.
Each variant boxes its payload to keep the enum compact. Type
mismatches (e.g. `ModelDelta` applied to a `GlyphArea`) are
silently ignored per the [`Applicable`](#applicablet) no-panic
rule.

### `GlyphAreaField` and `DeltaGlyphArea`

The per-field delta surface for `GlyphArea`: text,
scale, position, bounds, regions, outline, shape, zoom
visibility.

This is the granular surface any field-level
mutation reaches into. A font-size change is a
`GlyphAreaField::Scale(…)` inside a `DeltaGlyphArea` with an
`ApplyOperation` — that one pattern scales across every field
without bespoke plumbing per field.

`lib/baumhard/src/gfx_structs/area_fields.rs`
for the field enum and `OutlineStyle`; `area_mutators.rs` for
`DeltaGlyphArea`. The wrapper carries one `ApplyOperation`
shared across all fields in the batch, so "move this node 10
units right" and "set this node's text" use the same delta type
with different field lists and a different operation.

### `GlyphModelField`, `DeltaGlyphModel`, `GlyphModelCommand`

The `GlyphModel` mutation surface — the parallel of
the area-side delta and command trio, applied to composed-glyph
structures rather than plain text.

Everything the area side offers
([`GlyphAreaField`](#glyphareafield-and-deltaglypharea),
[`GlyphAreaCommand`](#glyphareacommand)), the model side needs
too — position nudges, matrix inserts and replacements, color
and font edits on individual components. Same operation vocab
(`ApplyOperation`), same `Applicable` dispatch, same walker
path; different target type.

`lib/baumhard/src/gfx_structs/model/mutator.rs`. `GlyphModelField`
variants cover the structural bits (matrix inserts, component
edits, model position). `DeltaGlyphModel` wraps them with an
`ApplyOperation`. `GlyphModelCommand` is the named-operation
counterpart for things that don't fit arithmetic — row pops,
matrix-coordinate moves, rotations. All three ride in the
`Mutation::ModelDelta` / `Mutation::ModelCommand` variants of
[`GfxMutator`](#gfxmutator).

### `GlyphAreaCommand`

The *named-operation* mutation surface, for actions
that are not arithmetic deltas.

Some operations have fixed semantics that
don't map to "add/subtract/assign": *pop the last three
graphemes*, *change the range of this region*, *delete a
specific region*. Commands are the vocabulary for those.
Imperatively named, grapheme-aware, covers ~16 operations.

`lib/baumhard/src/gfx_structs/area_mutators.rs`.
All grapheme-touching commands use `grapheme_chad` helpers, so
emoji / ZWJ / combining-mark sequences survive intact.

### `OutlineStyle`

A colored halo behind text, rendered as eight stamp
copies (four cardinals + four diagonals) around the main glyph.

When glyphs sit on a busy background, legibility
drops. `OutlineStyle` draws an outline halo so the glyphs read.
It is a field on `GlyphArea`, optional; default is no outline.

`lib/baumhard/src/gfx_structs/area_fields.rs`.
Two fields: `color: [u8; 4]` and `px: f32`. Cost is **9×** the
cosmic-text shapings of the area (one main + eight stamps). Hot
path, so enable only when background legibility demands it.

### `NodeShape`

A pluggable hit-test shape — `Rectangle` or
`Ellipse` — shared between the renderer SDF and the BVH
descent.

A node's visual silhouette and its clickable
silhouette must agree. `NodeShape` names the two today
(rectangle, ellipse) and gives both pipelines one source of
truth for "is this point inside?". Adding a new shape is
four small changes: one enum variant, one `style_spellings`
arm, one WGSL shader `case`, one `contains_local` arm. The
`style_spellings` `match` is exhaustive over the variants, so
the first change does not compile without the second.

`lib/baumhard/src/gfx_structs/shape.rs`.
`contains_local` does point-in-AABB or point-in-ellipse
(normalized coordinates, `nx² + ny² ≤ 1`); degenerate bounds
always return `false`. `intersects_local_aabb` supports
rect-select with conservative approximation for ellipses.

The format's shape vocabulary (`KNOWN_SHAPES`, published in
`format/enums.md` and enforced by `maptool verify`) is wider
than the variant set: `"hexagon"`, `"diamond"`,
`"parallelogram"` and `"rounded_rectangle"` are canonical but
have no shader case yet. `ShapeSpelling` is the pure
classifier that separates those from a genuine typo —
`Rendered` / `KnownNotYetRendered` / `Unrecognized` /
`Unspecified` — and `is_author_error` / `is_quiet_fallback`
are the two predicates it exposes for the reporting
decision. Before it existed, every hexagon in the demo map
warned on every load (issue #118).

`ShapeReport` is the second half of the split: the *routing*
as a value. `ShapeSpelling::report` composes the two
predicates into `Some(UnknownSpelling)` /
`Some(RectangleSubstituted)` / `None`, `ShapeReport::level`
is the single definition of which `log::Level` each carries,
and `from_style_string` does nothing but pick the literal
macro per arm — literal because `log`'s release compile-out
folds on a level the compiler can see, which a
`log::log!(computed, …)` would defeat. The value being a
value is what makes the decision ordinary testable data;
what is left, arm-to-macro, is a fact about source text and
`shape_tests.rs`'s `log_routing` reads it as one, holding
`from_style_string`'s body to a whitelist rather than
searching it for `log::` calls.

`KNOWN_SHAPES` is restated five times across three files —
three lists in `format/enums.md`, `LEGACY_SHAPE_ORDINALS` in
`crates/maptool/src/convert/enums.rs`, and the `shape_type`
ordinal table in `format/migration.md` — and each is pinned
back by a test that reads the restatement. The first four
derive their expectation from the constant directly; the
ordinal table is pinned to `LEGACY_SHAPE_ORDINALS` instead,
which is itself pinned to the constant, so the chain closes
in two hops rather than one. Either way, widening the
vocabulary is still one edit plus the copies the suite names
for you. The table in `shape.rs`'s `KNOWN_SHAPES` doc is the
index.

Shape-aware borders (glyph-drawn frames that follow
the ellipse outline, not just the AABB) wait on the
[`GlyphBorderConfig`](#border-geometry) side; the primitive
surface here is ready.

### `ZoomVisibility`

An optional inclusive `[min, max]` camera-zoom
window that gates whether an element renders at the current
zoom.

Visual detail that makes sense at one zoom
rarely makes sense at another. A legend label is precious when
zoomed in on its region and noise when the whole map is on
screen; an overview landmark is a guide when zoomed out and
redundant up close. `ZoomVisibility` lets authors say "this
appears between 1.5× and 3× zoom" and have the renderer silently
honor it — no script, no custom mutation, just two fields.

`lib/baumhard/src/gfx_structs/zoom_visibility.rs`.
Two `Option<f32>` fields, a `contains(zoom) -> bool` predicate;
cost is two branchless float comparisons, benchmarked as
sub-nanosecond. No cosmic-text reshaping or buffer-cache
invalidation fires on zoom steps. At the mindmap layer the
surface is two flat fields (`min_zoom_to_render`,
`max_zoom_to_render`) on `MindNode`, `MindEdge`,
`EdgeLabelConfig`, and `PortalEndpointState`; see
[`format/zoom-bounds.md`](./format/zoom-bounds.md) and
[§3: Zoom bounds](#zoom-bounds).

`NaN` zoom is treated as "not visible" deliberately —
a `NaN` camera is a bug upstream, and culling the frame surfaces
it faster than carrying the `NaN` through the glyph pipeline.
Inverted windows (`min > max`) render as "always hidden" at
runtime; `maptool verify` flags these as authoring errors.

The seam waiting here is **zoom-triggered LOD
mutations**: a `CustomMutation` bound to a zoom threshold could
swap a node's content entirely at the transition, so a cluster
summary becomes a detail view as you zoom in.
`GlyphAreaField::ZoomVisibility` already carries the mutator
target; what remains is the dispatcher that fires mutations on
zoom crossings.

### `Camera2D` and `CameraMutation`

A 2D canvas camera with pan/zoom and an intent-level
mutation vocabulary.

The renderer projects canvas coordinates to
screen pixels through a `Camera2D`; pan and zoom are represented
as `CameraMutation` variants so that one handler can accept
input, animation, and scripted values uniformly. When a gesture
says "pan by 10 pixels" and an animation says "fit-to-bounds with
5% margin", both go through the same apply site.

`lib/baumhard/src/gfx_structs/camera.rs`.
Position in canvas space (the point at the viewport center),
`zoom: f32` clamped between `MIN_ZOOM = 0.05` and `MAX_ZOOM =
5.0`. `CameraMutation` variants: `Pan { screen_delta }`,
`ZoomAt { screen_focus, factor }`, `ZoomCenter { factor }`,
`SetPosition { canvas_pos }`, `SetZoom { factor }`,
`FitToBounds { min, max, padding_fraction }`. Projection
helpers `canvas_to_screen` / `screen_to_canvas` are the only
place coordinate-space conversion lives.

### `Scene`

A multi-layer compositor: owns many `Tree`s at
different draw-order layers and screen-space offsets.

The mindmap canvas is one tree; connection
glyphs are another; the console overlay is another; the color
picker overlay is yet another. `Scene` collects them all,
orders them by layer, and provides a single `component_at(point)`
hit-test entry that walks top-to-bottom and returns the first
tree that owns the point. This is the structural seam where the
`AppScene` at the application layer
([§5: scene host](#appscene-and-scene-host)) attaches.

`lib/baumhard/src/gfx_structs/scene.rs`. Uses
`Slab<SceneEntry>` for stable ids across insert/remove; each
entry carries `layer: i32`, `offset: Vec2`, and `visible: bool`.
Hit-test is `O(trees)` at the scene level and `O(tree size)`
inside the matched tree.

### `TreeWalker`

The recursive dispatch engine that walks a
`MutatorTree` against a `Tree` and applies matched mutations.

Every mutation that ever lands on an element
goes through the walker — `MutatorTree::apply_to` just calls
`walk_tree_from`. The walker aligns children by channel (or by
position, depending on instruction), recurses, and dispatches
deltas to `Applicable::apply_to` at the leaves. Cost is `O(sum
of matching pairs)` — pruned branches are free.

`lib/baumhard/src/gfx_structs/tree_walker.rs`.
Key functions: `walk_tree_from` (the entry), `align_child_walks`
(the channel-based pairing), `process_instruction_node` (the
loop/spatial/map dispatch), `DEFAULT_TERMINATOR` (the closure
that resumes normal channel alignment after a `RepeatWhile`
exits). Branchless enough that matching-pair cost dominates.

### Mutator builder DSL — `MutatorNode`, `SectionContext`, `Repeat`, runtime holes

A serde-friendly AST (`MutatorNode`) that compiles
to a `MutatorTree<GfxMutator>` at apply time, with a
`SectionContext` for runtime value injection.

Declaring mutators by hand as
`MutatorTree<GfxMutator>` is fine for Rust code but hostile to
JSON authoring. The builder DSL solves this: authors write
`MutatorNode` in JSON (the shape is nearly identical to
`GfxMutator` but serializable and with `Repeat` for "N
consecutive channels with the same template"), and the builder
walks the AST with a `SectionContext` to resolve runtime values
(counts, fields, dynamically-chosen mutations) into a concrete
tree ready for `walk_tree_from`. This is the seam
[custom mutations](#4-the-mutation-framework) attach to.

`lib/baumhard/src/mutator_builder/`. The AST:
`MutatorNode::{Void, Single, Macro, Instruction, Repeat}`. The
indirection enums `ChannelSrc`, `CountSrc`, `MutationSrc` each
have a `Literal` variant (inline) and a `Runtime(String)` or
`SectionIndex` variant that consults the `SectionContext`
trait at build time. `build(ast, context)` returns a
`MutatorTree<GfxMutator>` with `Repeat` expanded to N children
on consecutive channels.

### Font system — `FONT_SYSTEM`, `AppFont`, `attrs_list_from_regions`, `RegionFamilies`, `rich_text_spans_from_regions`

A single global cosmic-text `FontSystem`, a
compile-time enum of available fonts, and a small set of bridges
from `ColorFontRegions` to the two cosmic-text shaping API shapes.

Every piece of text shaping in the project
flows through these. `fonts::init()` is called once at startup;
the `FONT_SYSTEM` `RwLock` is private and is acquired through
`acquire_font_system_write("site-name")` (or its `try_` / `_read`
siblings). Re-acquiring on the same thread panics at once; a wait on
another thread's hold is bounded and ends in a panic naming the
holder, never a hang. `AppFont` is generated at build time by scanning
`lib/baumhard/src/font/fonts/` — drop a font file in, recompile,
and the variant appears.

`lib/baumhard/src/font/`. Two bridges, one
shared private resolver, both live in `attrs.rs`:

- `attrs_list_from_regions` returns a single
  `cosmic_text::AttrsList` for callers using
  `Editor::insert_string`. `None` family resolution forces
  `Family::Monospace` per the `Editor` shape's existing fallback.
- `RegionFamilies` + `rich_text_spans_from_regions` return a
  `Vec<(&str, Attrs)>` for callers using `Buffer::set_rich_text`
  — the renderer's tree walker today. `RegionFamilies::resolve`
  caches the borrowed regions slice and pre-resolves family-name
  strings once per text area so the renderer's nine shape passes
  (one main + eight outline-halo stamps) reuse the same lookups.
  `None` family resolution omits the family pin (cosmic-text
  picks), preserving the walker's pre-existing fallback.

Unknown fonts log a `warn!` and drop to a monospace / no-pin
fallback rather than aborting — interactive paths must not panic
([`CODE_CONVENTIONS.md §9`](./CODE_CONVENTIONS.md)). The lock tells
its two failures apart rather than timing them: a thread
re-acquiring what it already holds is refused at once from a
thread-local record, and the 60-second timeout bounds only a wait on
*another* thread's hold, which is a deadlock if it runs that long.

### `RegionParams`, `RegionIndexer`, `RegionError`

A grid-bucketed spatial index over color/font
regions for cheap hit-testing.

Hit-testing "which region contains this point?"
against hundreds of spans over thousands of glyphs would be
linear per query. `RegionIndexer` divides the rendered surface
into a grid of buckets; queries consult the bucket containing
the point and scan only that bucket's regions. `RegionParams`
configures the grid, adapting to the resolution so dimensions
that don't factor cleanly (primes, near-primes) still get a
sensible subdivision.

`lib/baumhard/src/gfx_structs/util/`.
`RegionError::{InvalidParameters, Poisoned}` covers the failure
modes; callers match and decide rather than panicking. The indexer
and its parameters are a tested-but-unwired subsystem: they are
allocated by `Tree::new` but `MutatorTree::apply_to` does not
currently maintain them. Per-tree BVH descent (`Tree::descendant_at`)
handles hit-testing today; when the region index is wired, region
mutations must go through the mutator pipeline or the index will
drift silently.

### Animation primitives — `AnimationTiming`, `Easing`, `Followup`

The vocabulary for motion: a serializable timing
envelope on a mutation, an easing curve, and a reserved slot for
what happens after a run completes.

A "grow font" that snaps is fine; one that
animates reads better. Rather than a general scheduler, motion
here rides on the mutation that causes it — an authored
`CustomMutation` carries `timing`, and the runtime blends between
a pre-mutation and a post-mutation snapshot instead of executing
a separate program.

`lib/baumhard/src/mindmap/animation.rs` for the serializable
half (`AnimationTiming`, `Easing`, `Followup`, and the `lerp_*`
helpers); `src/application/document/types.rs` +
`animations.rs` for the app-side per-playback record and tick,
which live there because the snapshot type (`MindNode`) and the
completion commit do. See [§4](#animation-timing) for the
authoring surface and
[`format/animation-roadmap.md`](./format/animation-roadmap.md)
for what is wired versus dormant.

A second, generic vocabulary — `AnimationDef` / `AnimationInstance`
/ `Timeline` / `TimelineEvent` in `core/animation.rs` — was
deleted in #41. It had no implementers of its `Mutable` bound, no
callers, and an `update` signature that took its instance by value
and returned nothing, so it could not have driven a tick as
written. The long-form scheduling it stood for is the `Followup`
slot's job when that lands.

### Utilities — `grapheme_chad`, `color`, `geometry`

The shared-primitive toolkit: grapheme-aware text
operations, color types and macros, and epsilon-aware 2D
geometry helpers.

Three small modules that the rest of the
codebase builds on, rather than each module re-implementing its
own take:

- **`grapheme_chad`** — the only legitimate way to manipulate
  `String`/`&str` when the offset comes from user input.
  Functions: `count_grapheme_clusters`,
  `find_byte_index_of_grapheme`,
  `replace_graphemes_until_newline`, `split_off_graphemes`,
  `delete_back_unicode`, `delete_front_unicode`,
  `find_nth_line_grapheme_range`, `count_number_lines`, and the
  display-width pair `truncate_to_display_width` (clip the
  overflow) / `wrap_to_display_width` (fold it onto the next
  line). Byte slicing from user-facing counts lands mid-cluster
  on the first emoji; always reach for these. See
  [`CODE_CONVENTIONS.md §1`](./CODE_CONVENTIONS.md) and
  [`lib/baumhard/CONVENTIONS.md §B3`](./lib/baumhard/CONVENTIONS.md).
- **`color`** — `FloatRgba = [f32; 4]` and `Rgba = [u8; 4]`
  color types, `Palette = Vec<FloatRgba>`, plus compile-time
  macros `rgb!`, `rgba!`, and (non-const) `hex!`. Channel-index
  constants for consistency.
- **`geometry`** — `almost_equal` (`|a - b| ≤ 1e-5`, the
  baumhard-wide epsilon), `clockwise_rotation_around_pivot`,
  y-dominant `pixel_greater_than` and siblings (cursor-reading
  order), `vec2_area`. `Comparator` float equality uses
  `almost_equal`.

`lib/baumhard/src/util/`. All pure functions,
no shared state, no allocations beyond what the return types
demand.

---

## §3 The mindmap domain

The mindmap domain is the world of `.mindmap.json` — the on-disk
format and its in-memory mirror. It lives in
[`lib/baumhard/src/mindmap/`](./lib/baumhard/src/mindmap/) and is
documented schema-side under
[`format/`](./format/). The format references are authoritative
for field-by-field detail; this section is conceptual.

### `MindMap`

The document root: nodes, edges, canvas configuration,
palettes, custom mutations.

Everything a user can save and reload is here.
The `MindMap` is a plain serializable struct — no derived state, no
runtime caches. The loader deserializes it from JSON and carries any
key no field claims along untouched, so the save writes it back
([preserved unknown keys](#preserved-unknown-keys)); the
[canvas-role projection](#canvas-role-projection) and
[tree builder](#tree-builder) turn it into renderable form;
mutations transform it in place.
Helper methods (`children_of`, `all_descendants`,
`is_hidden_by_fold`, `is_ancestor_or_self`, `resolve_theme_colors`)
walk the data on demand rather than caching.

`lib/baumhard/src/mindmap/model/mod.rs`. The shape is a flat
`HashMap<String, MindNode>` (keyed by Dewey id), a
`Vec<MindEdge>`, a [`canvas: Canvas`](#canvas), a `palettes:
HashMap<String, Palette>`, and
`custom_mutations: Vec<CustomMutation>`. See
[`format/schema.md`](./format/schema.md) for the JSON surface and
[`format/README.md`](./format/README.md) for a minimum-viable
example.

### Preserved unknown keys

Every object in `.mindmap.json` is **open**: a key no field claims
neither fails the load nor disappears from the file. The loader warns
once, carries the key on the map, and writes it back at the next save.

The reason is version skew, in both directions at once. A map authored
by a newer build carries keys an older build has never heard of;
refusing the document leaves the reader with an empty window, and
Mandala is an editor — it loads the whole map, mutates it, and writes
the whole model back — so ignoring the keys deletes them at the next
save. Keeping them is what lets an older build open, edit and resave a
newer map without destroying the newer features. A typo
(`"min_zoom_to_rendr"`) is kept the same way, and named the same way,
because a key you still have is a key you can still fix;
[`maptool verify`](#maptool-cli) is where it becomes a nonzero exit.
The warning names the part of the document that carries the key —
`node "1.2"`, `edge[3]`, `palette "coral"` — rather than a byte
offset.

Mechanically it is **not** a per-type `#[serde(flatten)]` catch-all,
and could not be: `serde_json::Value` implements neither `Eq` nor
`Hash`, and the graph reachable from a load is full of types that
derive `Copy`, `Eq` or `Hash` (`Color`, `OrderedVec2`, `Position`,
`Anchor`, `Range`, `GlyphMatrix`). A catch-all on only the types that
can hold one would preserve keys in some places and drop them in
others. Instead `mindmap::unknown_keys` wraps the one parse a load
already pays for and records the route to every key serde handed to
`deserialize_ignored_any` — which covers the whole graph, including
types the source walk cannot see. What the model owes the mechanism is
only that no type absorbs a key first: no `deny_unknown_fields`, no
`flatten`, no `untagged` or `tag`. That set is not written down
anywhere — a hand-kept list is the twin surface
`lib/baumhard/CONVENTIONS.md` §B4 warns about — so
`lib/baumhard/src/util/serde_coverage.rs` walks baumhard's own sources
with `syn` and
`loader::tests::test_no_loadable_type_can_swallow_an_unknown_key`
fails the moment one appears.

**A derivation is a reading, and a reading can be wrong in a way the
document it feeds is wrong in too.** The same walk produces the list
of arrays `format/schema.md` publishes — the places a captured key's
route stops naming a node by its id and starts naming an element by
its index — and for a long time it produced that list by matching the
token `Vec` and one spelling of `#[serde(rename …)]`. Three live
arrays were missing as a result: `ColorFontRegions::regions` is a
`BTreeSet`, and `MutationListSrc::Literal` and
`GlyphModelField::GlyphLines` are newtype-variant payloads with no
field name. The document said the same three things, because it was
written by reading the walk, so nothing disagreed
([#122](https://github.com/Svendsys/mandala/issues/122)).

The walk now resolves a field's type through the index — a `type`
alias, a newtype struct, any of the eleven container spellings serde
writes as a growable array, and the bare slice `[T]` that `Cow<'a,
[T]>` bottoms out on. Four of those eleven do not keep the order the
file was written in at all: `HashSet` and `FxHashSet` iterate by hash,
`BTreeSet` by the element, `BinaryHeap` by its heap. That is a
*stronger* reason to publish them than a `Vec`, not a weaker one — a
save re-sorts them, so an index into one can move without anybody
having edited anything. The walk also reads every spelling serde
accepts for a member name, including the `deserialize` arm of a
list-form `rename`, a container `rename_all`, and the extra spellings
an `alias` admits. But the fix that matters is that the
walk is no longer the only witness.
`lib/baumhard/src/util/serde_probe.rs` asks the **generated**
`Deserialize` impls the same questions by handing them a
`Deserializer` that answers every request with the emptiest value
that satisfies it and records what was asked — no source text, no
`syn`, no list, and `serde_derive`'s own expansion as the authority
on both member names and array-ness.
`unknown_keys::tests::test_the_derived_positional_arrays_survive_an_independent_derivation`
and its member-name sibling fail when the two disagree. Neither is a
replacement for the other: the probe cannot see a serialize-only
proxy, and the walk cannot see through a generic instantiation. Their
disagreement is the signal. Both see `#[serde(alias = "…")]` and both
publish it — the derive writes a field's aliases into the same
`FIELDS` list as its name and a variant's into `VARIANTS` — because a
file may write any accepted spelling and a captured route is recorded
against whichever it wrote.

Openness is about **keys, not meanings**. An `edge_type` the renderer
does not know still loads — open vocabularies stay open — and semantic
violations (an edge pointing at no node, a `color_schema` naming a
palette that is not there) are [`maptool verify`](#maptool-cli)'s
business. The interiors of `macros` / `inline_macros` are deliberately
opaque and are never reported. Three legacy spellings are the
exception and still refuse the load: a top-level `portals`, per-node
`text` / `text_runs`, and a non-array `sections` are names the current
model means something else by, and each has a `maptool convert` verb.

`lib/baumhard/src/mindmap/unknown_keys.rs`,
`lib/baumhard/src/mindmap/loader.rs`,
`lib/baumhard/src/util/serde_coverage.rs`,
`lib/baumhard/src/util/serde_probe.rs`. See
[`format/schema.md`](./format/schema.md) §"Unknown keys are kept" for
the policy as authors read it, and
[`format/validation.md`](./format/validation.md) for the split between
what the loader reports and what `verify` fails on. Its severe
counterpart is [skipped constructs](#skippedconstruct--skippedconstructs),
below.

### `SkippedConstruct` / `SkippedConstructs`

One construct a load could not read at all, lifted out of the document
so the rest of the map opens, and written back untouched at the next
save. `SkippedConstructs` is the ordered collection of them, carried
on `MindMap` beside `unknown_keys` and with the same `#[serde(skip)]`:
they go back at their own routes, not into a side object.

The problem is the acute form of the one
[preserved unknown keys](#preserved-unknown-keys) solves. A key from a
newer build is **inert** — nothing reads it, so ignoring it changes
nothing about what the map does. A **variant** from a newer build is
the opposite: it *is* the instruction. `{"mutator": {"Glow": …}}` used
to make the whole document unloadable, so opening a newer map in an
older build gave an empty window and the newer feature was one
accidental save away from gone.

**The unit is the whole construct**, never the part inside it that
failed, and that is the load-bearing design decision. Dropping one
`Mutation` out of a macro would leave a custom mutation that still
appears in `mutation list`, still fires, and now does two of the three
things it says it does — a silent partial behavior with nothing to
see. So the unit is the nearest container whose absence reads *as*
absence: a whole `custom_mutations[i]`, a whole node
`inline_mutations[i]`, or a whole node-or-section
`trigger_bindings[i]`. Nothing else is skippable — a node, an edge,
the canvas or a palette this build cannot read still fails the load,
because a map missing part of itself with no sign of which part is
worse than no map.

**Refused for any reason, not only an unknown variant.** Sorting
serde's message into "from the future" and "a typo" would mean parsing
that message — the twin surface `lib/baumhard/CONVENTIONS.md` §B4 is
about — and it sorts on the wrong axis anyway: the load's question is
"can I carry this out?", and the answer is no in both cases. So the
load skips what it cannot read, warns saying what serde said and that
nothing the construct describes will run, and `maptool verify` reports
every one as an `unknown_variant` violation with a nonzero exit. The
two questions — *can I open this?* and *is this file right?* — stay
separate.

Ordering matters at save: captured keys are spliced back **before**
skipped constructs, because a key's positional route was resolved
against the array with the constructs already lifted out. See
`loader::to_json_value`.

`lib/baumhard/src/mindmap/unknown_keys.rs` (the types),
`lib/baumhard/src/mindmap/loader.rs`
(`load_skipping_unreadable_constructs`,
`excise_unreadable_constructs`),
`crates/maptool/src/verify/unknown_keys.rs`
(`check_skipped_constructs`). See
[`format/schema.md`](./format/schema.md) §"Unknown keys are kept" and
[`format/validation.md`](./format/validation.md) §"Unknown variants".

### `Canvas`

The per-map shared rendering context: background
color, default node and connection styles, live theme-variable
map, named theme presets.

Some things are per-map rather than per-node:
the canvas background color, the defaults nodes and edges fall
back to when their fields are absent, the `var(--name)` theme
variables colors reference, and the presets theme-switching
mutations copy into those live variables. `Canvas` is that
shared state. It sits on `MindMap` directly (`canvas: Canvas`)
and is consulted at scene-build time for defaults and theme
resolution.

`lib/baumhard/src/mindmap/model/canvas.rs`. Key fields:
`background_color`, default-style records for nodes and
connections, `theme_variables: HashMap<String, String>` (live
values), `theme_variants: HashMap<String, HashMap<String,
String>>` (named presets). The
[`SetThemeVariant`](#document-actions) document action copies
a preset into the live map;
[`SetThemeVariables`](#document-actions) patches individual
entries.

### `MindNode`

One node — position, size, style, layout hint,
palette binding, channel, trigger bindings, and one or more
[`MindSection`](#mindsection)s carrying the text content.

The unit of content. Each node renders as a
shape with one or more text-bearing **sections** inside,
optionally framed by a glyph border, and participates in the
parent-child tree through its `parent_id`. Post-section refactor
the node owns visual chrome (background, frame, shape, border,
shadow) and structural pieces (`channel`, `color_schema`,
`trigger_bindings`, `inline_mutations`, zoom-bounds); the
user-typed text lives on its sections.

`lib/baumhard/src/mindmap/model/node.rs`. Full
field reference in [`format/schema.md`](./format/schema.md). Author
owns non-overlap of node AABBs; the model does no collision
checking. The tree builder excludes folded subtrees from the
display tree; the underlying data persists either way. The node
container materializes as a chrome-only `GfxElement::GlyphArea`
in the runtime tree, with the section subtree appended as
children — see [tree builder](#tree-builder).

### `MindSection`

A positioned text-bearing surface inside a
`MindNode` — the post-section data shape's home for `text` and
`text_runs`. Every renderable node has at least one section.

The user-facing strata of data. A node is a
*container*; a section is *what the user typed*. Sections give
the architecture room to grow per-stratum styling, per-stratum
mutations, and per-stratum interaction without making the node
itself bigger. For migrated maps the typical shape is one default
section per node (offset `(0, 0)`, fills the parent); authors who
want multiple strata of data on one node opt in by appending
extra sections.

`lib/baumhard/src/mindmap/model/node.rs:MindSection`.
Plain data — `text`, `text_runs`, `offset`, optional `size`,
`channel`. In the runtime tree each section becomes a
`GfxElement::GlyphArea` child of the owning node's container area,
plus a structural `GfxElement::GlyphModel` grandchild that exists
as a future per-component-mutation seam (the renderer skips it).
The renderer's tree walker shapes each section-area into its own
`cosmic_text::Buffer` keyed by `unique_id`, so multiplicity falls
out for free. Loader rejects pre-section maps with a concrete
pointer at `maptool convert --sections`. Full reference:
[`format/sections.md`](./format/sections.md).

### `MindEdge`

A directed connection between two nodes — line-mode
or portal-mode — with style, optional label, and optional
per-endpoint state.

Edges carry both hierarchical structure (when
their `type` is `parent_child`) and arbitrary cross-links (when
`type` is `cross_link`). They render as either a path of glyphs
along a Bézier curve (line mode) or a pair of small markers, one
at each endpoint (portal mode). A line-mode edge can have a
single text label sitting along the path; a portal-mode edge has
two endpoint records, each with its own text and styling.

`lib/baumhard/src/mindmap/model/edge.rs`. Edges
have **no stable id** — they are identified by the tuple
`(from_id, to_id, edge_type)`
([`CODE_CONVENTIONS.md §3`](./CODE_CONVENTIONS.md)). The
`display_mode` field switches rendering style without changing the
underlying edge identity; flipping a long edge from line to portal
is a one-field change. Field reference in
[`format/schema.md`](./format/schema.md).

Multiple edges between the same pair with different
`type` are allowed (rare but legitimate). Multiple edges with the
*same* tuple are a duplicate and a validation error.

### Dewey-decimal IDs

Dot-separated hierarchical node IDs (`"0"`, `"1.2"`,
`"1.2.3"`) that encode tree structure in the key itself.

Reading a `.mindmap.json` reveals the tree shape
in the keys. IDs sort as numbers segment-by-segment (`"1.10"` after
`"1.9"`, not before), and `derive_parent_id` recovers the parent
without pointer chasing. The format is human-friendly and
diff-friendly — exactly the sort of place where opaque UUIDs would
have ended in the same byte count and zero readability.

`lib/baumhard/src/mindmap/model/mod.rs`.
`id_sort_key` extracts the last segment for sibling sort;
`derive_parent_id` strips it. Fresh IDs are minted by
`fresh_child_id` in `src/application/document/topology.rs`
without reusing deleted gaps. Full reference:
[`format/ids.md`](./format/ids.md).

IDs do **not** cascade on runtime reparent — when
node `"1.2"` moves under `"0"`, it stays `"1.2"` and `parent_id`
becomes the truth. They *do* cascade on delete-with-orphan-promote.
This trade keeps reparent cheap; `maptool verify` flags drift.

### Channels (mindmap level)

The `MindNode.channel` field — the user-facing
surface of the Baumhard routing tag.

Authors tag siblings with channels to opt them
into selective mutations. A `CustomMutation` whose mutator targets
channel 1 hits only siblings tagged 1; siblings tagged 0 are
skipped. Multiple siblings can share a channel (broadcast group),
or each can be unique (per-sibling targeting). All existing maps
default to channel 0 and behave as if the field did not exist.

Stored as `usize` on `MindNode`; preserved
through tree builder onto the corresponding `GfxElement.channel`;
consulted by [`BranchChannel`](#channel-and-branchchannel) at walk
time. Full reference:
[`format/channels.md`](./format/channels.md).

A `TargetScope::ChildrenOnChannel(n)` variant is the
named extension waiting on this field — it would let a mutation
declare "children whose channel is 1" without an inline predicate.

### Palettes

Map-level named color schemes; nodes reference them
through `color_schema { palette, level, … }` rather than carrying
colors inline.

The legacy miMind format stored full palette
data on every node; the testament map alone duplicated the same
~225 palettes across nodes. Hoisting palettes to the document
level is a 100× reduction in file size and turns "rethemes the
whole map" into a single edit. Each palette is an array of
`ColorGroup`s indexed by depth; a node's `level` is which group
it pulls from. Level-clamping (last group when out of range) makes
deep subtrees degrade gracefully.

`lib/baumhard/src/mindmap/model/palette.rs`, with the cascade
itself in `model/theme.rs`. A node's binding lives in its
optional `color_schema` field, a `ColorSchema` record with
`palette: String` (the key into `map.palettes`),
`level: usize` (which `ColorGroup` to pull from), two
flags — `starts_at_root` (does level 0 apply to the schema
root or to its children?) and `connections_colored` (do edges
leaving this node inherit the palette stroke color?) — and
`overrides`, a `ColorOverrides` record of four optional strings
naming this node's own exceptions to the group.
`resolve_theme_colors` on `MindMap` does the lookup;
out-of-range `level` clamps to the last group rather than
failing, and `starts_at_root: false` leaves the schema root
itself unresolved so its own `style` stands. Validation
requires every referenced palette to exist with at least one
group.

The cascade is **override first, palette second, `style`
third**, and the
projection passes read it through four sibling helpers —
`node_background_color`, `node_frame_color`,
`node_text_color`, `node_title_color` — plus
`edge_theme_stroke_color`, which supplies the themed tier of
[`MindEdge`](#mindedge)'s color cascade for an edge whose
source node sets `connections_colored`. That helper reads
`node_frame_theme_tier` — the same one the border ladder
reads — so an `overrides.frame` reaches an edge exactly as it
reaches the node's own frame, and an empty frame is a hole on
both. Anything more specific
than the node still wins: a `TextRun` naming its own color, a
`border.color` override, a per-edge `glyph_connection.color`.

The `overrides` tier is what makes the theme *editable per
node*. A direct "make this one green" is more specific than an
inherited theme and has to win, and it cannot land in `style`:
`style` is the tier the palette shadows, and every migrated
node carries baked `style` colors that are stale copies of its
own theme, so a `style` write would report success and change
nothing on screen. `set_node_bg_color` /
`set_node_border_color` / `set_node_text_color` therefore write
`color_schema.overrides` on a themed node and `style` on an
unthemed one, through one shared
`set_node_color_channel`; `UndoAction::EditNodeStyle` carries
`before_color_schema` so undo puts the node back on its
palette.

A tier you can write is only half a tier: each of the three
setters takes an `Option`, and `None` — what `color bg=reset`
and the empty string on a non-fill channel both resolve to —
*drops* the override so the group shows through again, rather
than storing the authoring literal that would pin the node off
its theme forever. On an unthemed node there is nothing below
`style`, so the same `None` names the authoring default
instead. Full reference:
[`format/palettes.md`](./format/palettes.md).

Animated palette transitions are the seam — the data
shape is already mutation-friendly; the runtime would need to
interpolate `ColorGroup` fields on a clock.

### Text runs

Non-overlapping styled character ranges within a
node's text — bold, italic, underline, font, size, color,
hyperlink.

A single node can have rich text without being
fragmented into multiple nodes. Text runs are the mindmap-side
surface that the renderer translates into `ColorFontRegions`
spans for shaping. The user-visible effect is a per-span
override: emphasis on the first word, a colored annotation in
the middle, a link at the end — all on one node.

`lib/baumhard/src/mindmap/model/node.rs`.
Each run carries `start`, `end`, `bold`, `italic`, `underline`,
optional `font`, optional `size_pt`, optional `color`, optional
`hyperlink`. Indexed by **grapheme clusters** — what users see
as one character — matching `ColorFontRegions::Range` and
baumhard's text primitives (see
`lib/baumhard/CONVENTIONS.md §B1` and the
[`Range`](#range) entry above). Cluster indexing keeps a run
that ends after a Hebrew niqqud combining mark or a ZWJ-emoji
family on the same boundary the cosmic-text bridges in
`baumhard::font::attrs` slice on, so per-region styling lands
on whole glyphs. Validation: non-overlapping, ascending,
`end <= text's grapheme-cluster count`. Uncovered ranges
inherit the node-level style. Full reference:
[`format/text-runs.md`](./format/text-runs.md).

If `text_runs` is non-empty, **only covered ranges
render** — uncovered graphemes drop silently. So authors must
cover every grapheme they want visible, not just the ones they
want to restyle. This is by design (it simplifies the
renderer's region pass) but it is the single biggest trap in
the format; `maptool verify` does not catch partial-coverage
intent vs. accident.

### Theme variables

Document-level CSS-style named colors referenced as
`var(--name)` from any color field.

Avoids hex repetition across hundreds of nodes
and edges. A theme switch changes the variable; everything
referencing it updates. Theme variants (presets) can be stored
under `canvas.theme_variants` and applied through the
`SetThemeVariant` document action.

Resolved at scene-build time in the color
cascade — variable lookup, then fall through to a default if the
name is unknown. Document actions
[`SetThemeVariant`](#document-actions) and `SetThemeVariables`
mutate the live `canvas.theme_variables` map.

### Zoom bounds

The mindmap-level surface of
[`ZoomVisibility`](#zoomvisibility): two flat fields
(`min_zoom_to_render`, `max_zoom_to_render`) on every renderable
entity (`MindNode`, `MindEdge`, `EdgeLabelConfig`,
`PortalEndpointState`). Cascade rule (replace-not-intersect),
field semantics, and authoring shape:
[`format/zoom-bounds.md`](./format/zoom-bounds.md). Console-verb
authoring: `zoom min=1.5 max=3.0`, `zoom clear`, `zoom max=unset`
against the active selection.

### Border geometry

Glyph-drawn frames around nodes — Unicode box-drawing
characters laid out around the node's AABB.

Borders are the visual frame that gives a node
its "boxed" appearance. They are made of glyphs (light, heavy,
double, rounded, or fully custom box-drawing chars), not solid
strokes — consistent with the
[everything-is-glyphs](#everything-is-glyphs) invariant. Borders
also serve as anchor surfaces for portal endpoints, which sit at
parametric positions along the border perimeter.

`lib/baumhard/src/mindmap/border.rs`. The
`GlyphBorderConfig` per-node record (in
`lib/baumhard/src/mindmap/model/node.rs`) carries:

- `preset: String` — one of `"light"` (`─ │ ┌ ┐ └ ┘`),
  `"heavy"` (`━ ┃ ┏ ┓ ┗ ┛`), `"double"` (`═ ║ ╔ ╗ ╚ ╝`),
  `"rounded"` (`─ │ ╭ ╮ ╰ ╯`, the default), or `"custom"`.
- `font: Option<String>` — font family override; `None` =
  system default.
- `font_size_pt: f32` — glyph size.
- `color: Option<String>` — `#RRGGBB` override; `None` =
  inherit from `style.frame_color`.
- `glyphs: Option<CustomBorderGlyphs>` — per-side glyph
  overrides (top / bottom / left / right / four corners); only
  consulted when `preset = "custom"`.
- `padding: f32` — border-to-content gap in pixels.

Geometry constants (`BORDER_CORNER_OVERLAP_FRAC`,
`BORDER_APPROX_CHAR_WIDTH_FRAC`) are shared between the
renderer and tree builder; they must agree, or corner
alignment drifts.

Borders today only render on rectangular nodes
(`NodeShape::Rectangle` and `style.show_frame = true`). Ellipse
borders need shape-aware glyph layout — a named seam not yet
implemented.

### `GlyphConnectionConfig`

The per-edge rendering configuration: body glyph,
caps, font, font size, screen-space font clamps, color.

Every `MindEdge` carries one. `GlyphBorderConfig`
is to a node what `GlyphConnectionConfig` is to an edge: the
shape of the glyphs that draw the thing. The body glyph is
repeated along the connection path; `cap_start` and `cap_end`
override the terminal glyphs if present. Font size is
interpreted as the target *on-screen* size at zoom = 1.0;
`min_font_size_pt` and `max_font_size_pt` clamp the effective
screen-space size as the camera zooms, so a long edge stays
readable both zoomed in and zoomed out.

`lib/baumhard/src/mindmap/model/edge.rs:335+`.
Fields: `body: String` (default mid-dot `·`), `cap_start` /
`cap_end: Option<String>`, `font: Option<String>`, `font_size_pt:
f32`, `min_font_size_pt` / `max_font_size_pt: Option<f32>`,
`color: Option<String>`. Color cascade priority (highest
first): edge-label → `glyph_connection.color` → the source
node's frame tier (`overrides.frame`, else its palette group's
`frame`) → `canvas.default_connection.color` → `edge.color`.
The theme sits above the canvas default because it is per-node
and the default is map-wide, and the frame tier is the one
`MindMap::node_frame_theme_tier` also hands the node's border
resolver, so a per-node frame override moves both.
`effective_font_size_pt(zoom)` is the helper callers reach for
to derive the clamped screen-space size.

### `ControlPoint`

An author-set Bézier offset on a `MindEdge`,
expressed as an offset from a node center rather than an
absolute canvas coordinate.

Straight line-mode edges can become curved
when the author specifies control points. Zero control points
is a straight segment; one promotes to a cubic Bézier (via
quadratic-to-cubic lifting); two or more define a cubic
directly. Control points live as offsets from endpoint centers
so a node move drags the curve along without the author
having to re-tune the path.

`lib/baumhard/src/mindmap/model/edge.rs`. Consumed by
[connection path construction](#connection-paths), where
`build_connection_path` converts control points from offsets
into cubic control coordinates in canvas space.

### Portals

Edges with `display_mode = "portal"`: rendered as two
glyph markers, one at each endpoint, instead of a connecting
line.

When two endpoints are far apart on the canvas,
drawing a literal line between them is visually noisy and
expensive (hundreds of glyphs). Portals decouple the visual link
from the physical span: the user sees a small glyph at each end,
recognizes them as a pair (matching color, matching text), and
can double-click either to fly the camera to the partner.
Portals share the underlying edge with line-mode — the only
difference is `display_mode`.

`lib/baumhard/src/mindmap/model/edge.rs`. Per-endpoint state lives
in `PortalEndpointState`: `color`, `border_t` (parametric
position on the owning node's border), `perpendicular_offset`
(signed distance along the outward normal), `text`, `text_color`,
`text_font_size_pt`, `text_min/max_font_size_pt`,
`min/max_zoom_to_render`. The icon and the adjacent text are
sibling leaves of the portal tree, so a click resolves to exactly
one of them through the tree's BVH
(`AppScene::portal_at` → `PortalHitIndex::resolve`, yielding a
`PortalHit` that names the sub-part). A click on the icon selects
`SelectionState::PortalLabel` (and font/color ops target the icon
channel) while a click on the text selects
`SelectionState::PortalText` (and ops target the text channel).
An endpoint with no text lays its text slot out at zero extent, so
the reserved slot cannot answer a click.
Full reference: [`format/portal-labels.md`](./format/portal-labels.md).

### Edge labels

Optional text along a line-mode edge, positioned by
parametric `t` along the path with an optional perpendicular
offset.

Edge annotations — "depends on", "blocks",
"derived from". Line-mode only; portal edges use per-endpoint
text instead. Labels can be dragged to reposition (native today),
authored via the `label position_t=… perpendicular=…` console
verb (cross-platform), and given their own zoom-window override.

`EdgeLabelConfig` on `MindEdge`. Position
encoded as `(position_t, perpendicular_offset)`. Drag computes
both via `closest_point_on_path`. Replace-not-intersect zoom
cascade matches portals.

### Connection paths

The geometric backbone of edge rendering: straight
segments and cubic Bézier curves with anchor resolution at the
endpoints.

Given two node AABBs and optional control
points, compute the curve along which to lay out edge glyphs.
The same path math powers glyph placement (sample at uniform arc
length), label drag (project cursor onto path), and hit-testing
(distance from cursor to path).

`lib/baumhard/src/mindmap/connection/`. Key
functions: `build_connection_path` (from anchors + control
points), `resolve_anchor_point` (auto / top / right / bottom /
left), `point_at_t`, `tangent_at_t`, `closest_point_on_path`
(uniform-t sampling + Newton refinement for cubics, direct
projection for straight lines), `sample_path` (arc-length-uniform
glyph placement), `distance_to_path`, and the pair behind the edge hit-test —
`distance_to_path_within`, which `hit_test_edge` calls, and
`path_bounds`, which it calls in turn. `path_bounds` is the
control polygon's AABB, which contains the curve by the Bezier
convex-hull property *in real arithmetic*; the f32 escape from
that box is what `distance_to_path_within`'s slack allows for, and
it is the reason to reach for that function rather than to compare
against the box by hand. Quadratics get promoted to
cubic at build time, so the apply path is always one of two
shapes.

### Portal geometry

The conversion between `border_t ∈ [0, 4)` and a
canvas point on a rectangular node's border, plus directional
defaults.

Portal endpoints must sit on their owning
node's border, parametrically — so when the node is resized, a
label at "the middle of the right edge" stays at the middle of
the right edge. The side-indexed encoding (`[0, 1)` = top,
`[1, 2)` = right, `[2, 3)` = bottom, `[3, 4)` = left) is the right
abstraction: stable across resize, deterministic across corners.

`lib/baumhard/src/mindmap/portal_geometry.rs`.
Functions: `wrap_border_t` (rem-Euclid into `[0, 4)`),
`border_point_at`, `border_outward_normal`,
`default_border_t` (the auto-orientation: cast a ray from owner
to partner center), `nearest_border_t` (project a canvas point to
the closest border parameter, used by drag-snap).

### Fold state

A boolean per node; folded subtrees are excluded
from the display tree but persist in the model.

Hide subtrees without losing data. The user
can collapse a region of the map; reopening restores it. Each
build computes the hidden set once with
`MindMap::fold_hidden_set` (an O(N) pass) and tests membership
per element; the tree builder passes a `parent_folded` flag down
the recursive walk so the same cascade is checked by construction.
One-off callers can still use `MindMap::is_hidden_by_fold`, which
walks the parent chain at O(depth) per call.

### Tree builder

Projects a `MindMap` into a Baumhard
`Tree<GfxElement, GfxMutator>` mirroring the parent-child
structure, with each `MindNode` materializing as a three-deep
subtree (container + section-areas + section-models).

Mutations need a `Tree` to walk against. The
tree builder constructs it from the model: each visible
`MindNode` becomes a chrome-only container `GfxElement::GlyphArea`
plus one `GfxElement::GlyphArea` per section (carrying the
section's text + theme-resolved regions) plus a structural
`GfxElement::GlyphModel` grandchild per section-area as a
future-mutation seam. Parent-child relationships become tree
edges; channels are preserved. Per-role sub-builders cover
borders, portals, connections, edge labels, and edge handles,
each producing its own tree (and matching mutator-tree) so
per-role mutations stay scoped.

`lib/baumhard/src/mindmap/tree_builder/mod.rs`. Returns a
`MindMapTree` with `node_map: HashMap<String, NodeId>` (mind id →
container arena id), `section_map: HashMap<(String, usize), NodeId>`
(mind id + section index → section-area arena id), reverse maps
for both, and an `owning_mind_id` helper that climbs up to three
arena edges (model → section → container) to find the owning
mind-node. Section-areas (and section-models) carry
`Flag::SectionRoot`. Folded nodes are excluded.

### Canvas-role projection

Projects a `MindMap` into one `Tree<GfxElement, GfxMutator>` per
canvas role — nodes, borders, connections, connection labels,
portals, section frames, and the three handle families. There is
no flat intermediate scene: the walker in
`src/application/renderer/tree_walker.rs` is the only consumer,
and every role reaches it as a tree.

Each role file under `lib/baumhard/src/mindmap/tree_builder/`
follows the same **courier** shape:

1. a *data pass* that resolves the model plus this frame's
   [overrides](#frame-overrides) into plain per-element data —
   `border_node_data`, `build_connection_elements`,
   `build_label_elements`, `portal_pair_data`,
   `build_section_frames`, `build_selected_node_handles`,
   `build_selected_section_handles`;
2. a *full-rebuild projection* (`build_*_tree`) and an *in-place
   projection* (`build_*_mutator_tree`) over that data.

Which projection runs is decided by hashing the data pass's
output, because any interaction re-runs every role: clicking an
edge label reaches the border and portal passes too. Two
questions, and roles that can answer both ask both. The
*structural* signature (`border_structure_signature`,
`portal_structure_signature`, `connection_identity_sequence`,
`handle_identity_sequence`, …) covers only what fixes the tree's
channel layout, and answers *can a mutator align against the
registered arena at all?* — a color change must leave it alone,
or a color-picker hover would reallocate the arena every frame,
which is the cost the in-place path exists to avoid. The
*content* signature (`border_content_signature`,
`portal_content_signature`) covers every field either projection
reads, and answers *is there anything left for that mutator to
write?* On a content match the role does nothing: the registered
tree already holds what a rebuild would produce. Completeness
runs the opposite way for the two — a structural signature is a
deliberate subset, a content signature that misses a field
serves a stale frame, so each content hasher destructures its
element type exhaustively and a new field fails to compile until
it is accounted for. Section frames, which have no in-place
projection, carry only the content signature
(`section_frame_content_signature`); the connection, label and
handle roles carry only the structural one and always apply
their mutator.

Those three still spell theirs `*_identity_sequence` and return
a `Vec` the caller hashes through `hash_canvas_signature`, which
is the older shape of the same idea; borders and portals were
converted to stream into the hasher when their content
signatures landed. The two spellings mean the same thing and the
conversion of the remaining three is unclaimed work, not a
distinction.

Style is resolved exactly once, in the data pass; neither
projection re-resolves it. Two inputs cross role boundaries.
`node_clip::node_clip_aabbs` is the one the connection sampler
clips glyph samples against; it reads only the resolved border
`font_size_pt`. [`EdgePathCache`](#edge-path-cache) is the other:
the sampler, the selected edge's grab-handles and the label layout
all want the same edge's `ConnectionPath`, and it is built once per
edge per frame for whichever of them asks first.

The application layer drives the sequence through
`CanvasFrame` (`src/application/app/scene_rebuild.rs`), which owns
the shared per-frame inputs (the fold-hidden set, the assembled
overrides) and exposes one `update_*` per role so a caller can
refresh only the roles its interaction can change — a scroll-wheel
zoom touches connections, labels, portals, and edge handles but
never a border or a resize handle.

Selection highlight, drag-preview offsets, NodeEdit dimming, and
color-picker previews are all applied at projection time and never
committed back to the model. Caching at the
[`scene_cache`](#scene-cache) level reuses sampled connection
positions when endpoints don't move.

### Frame overrides

`tree_builder::overrides` holds the transient, frame-local
substitutions the projection folds on top of the committed model:
`SceneSelectionContext` (selection plus the inline label / portal
text editors' uncommitted buffers), `EdgeColorPreview` /
`PortalColorPreview` (color-picker hover), `BorderPreview` (staged
`border preview …` edits), and `InteractionModeOverrides` (the
mode-derived resize / NodeEdit targets). `FrameOverrides` bundles
all four; `MindMapDocument::frame_overrides` assembles one per
rebuild so no two roles can disagree about what the user is
pointing at.

### Scene cache

A per-edge cache of sampled glyph positions, keyed
on `(from_id, to_id, edge_type)`, reused across frames when
endpoints have not moved.

Sampling a cubic Bézier path at uniform arc
length is the most expensive per-edge work. The cache invalidates
on endpoint drag (via `drag_offsets`) and on zoom or structural
change. Otherwise the previous frame's samples are reused with a
cheap `point_inside_any_node` clip filter.

`lib/baumhard/src/mindmap/scene_cache.rs`.

An entry holds the sampled geometry, the endpoints it was sampled at,
and the `SampleParams` — effective font size, glyph spacing, per-path
sample budget — it was sampled under. It holds nothing the frame
*draws* with, and there is no exception: body glyph, cap glyphs, font
family, font size and body color are all read from the live model on
every path, so no styling edit can be served stale. The two reuse
doors, `reusable` and `reusable_mut`, compare the whole
`SampleParams`, so a font-size or spacing edit resamples without the
mutating site having to remember `clear()`.

What the caller still owes is geometry the params cannot see, and only
that: an endpoint that moved in the model without appearing in the
drag `offsets` map, an anchor or control-point edit, and a node
resize. Nothing about color is on that list — neither a theme-variable
edit nor a direct `edge.color` edit.

`refill` is the cache's only writer, and it hands the sampler the
buffer the entry already holds rather than a new one, so an edge that
resamples while cached allocates nothing. A refill that fills nothing
evicts, so an entry with no points — which the reuse doors would
otherwise serve as an edge that draws nothing, forever — cannot
exist.

Liveness is a generation stamp rather than a per-frame set of keys:
`begin_pass` opens a pass, every route that hands out or writes an
entry stamps it, and `evict_unseen` drops what still carries an older
stamp. Reaching an entry *is* marking it, so a route cannot draw an
edge and then forget to record that it saw one.

### Edge path cache

A per-frame memo of `ConnectionPath`s, keyed by index into
`map.edges` and filled on first ask.

Three passes want the same edge's path in one rebuild: the
connection sampler on a scene-cache miss, the label layout for
every labeled edge, and the grab-handles for the selected edge.
Each used to build its own.

It is **lazy** rather than precomputed, which is the load-bearing
half: an unlabeled, unselected edge that hits the scene cache asks
for no path and none is built. It borrows the `MindMap` and the
drag-offset map it resolves against, so a memo cannot outlive the
inputs it is keyed on — nothing has to remember to invalidate it.
`CanvasFrame` owns one per rebuild and threads it through both
connection passes.

`lib/baumhard/src/mindmap/tree_builder/edge_path.rs`.

### Trigger bindings

Per-node bindings of input events to custom-mutation
ids: `OnClick`, `OnHover`, `OnKey`, `OnLink`.

Authoring interactive map elements without
custom code: a button-like node fires a custom mutation when
clicked. Bindings carry an optional context filter (Desktop /
Web / Touch); empty means "all platforms".

`MindNode.trigger_bindings`; dispatch lives
in the application's input handlers. Missing mutation IDs are
silent no-ops — runtime ignores rather than panicking.

---

## §4 The mutation framework

The mutation framework is the primary extensibility seam. It spans
both crates — the AST and walker live in Baumhard, the registry
and dispatch live in Mandala — and is the answer to "how do I add
behavior to a mindmap without recompiling?" Everything from
"grow font 2pt on the selected subtree" to size-aware layouts like
`flower-layout` and `tree-cascade` flows through it.

For the JSON authoring surface see
[`format/mutations.md`](./format/mutations.md). For the
prescriptive carrier shape see
[`lib/baumhard/src/mindmap/custom_mutation/`](./lib/baumhard/src/mindmap/custom_mutation/).

### `CustomMutation`

The carrier struct: an id, name, description,
contexts, optional mutator AST, target scope, behavior, optional
document actions, and optional animation timing.

A `CustomMutation` is one named, reusable
operation. Authored as JSON (declarative) or registered in Rust
(imperative); referenced by id from console verbs, trigger
bindings, or other custom mutations. The same shape covers tiny
deltas ("add 2pt to font") and structural algorithms
(`flower-layout`).

`lib/baumhard/src/mindmap/custom_mutation/mod.rs`. Fields: `id`
(unique key), `name` and `description` (human-readable),
`contexts` (taxonomy tags — see
[contexts](#contexts-taxonomy)), `mutator` (an optional
`MutatorNode` AST), `target_scope` (which nodes the change
covers), `behavior` (`Persistent` or `Toggle`),
`document_actions` (optional canvas-level operations), `timing`
(optional `AnimationTiming`).

### Four-source loader

Mutations merge from four sources at startup, with
ascending precedence: App < User < Map < Inline.

Authors at every layer can define mutations
without stepping on each other. A bundled "grow-font-2pt" can be
overridden by the user's personal version, which can in turn be
overridden by a map's local definition, which can in turn be
overridden by a single node's `inline_mutations`. The
`MindMapDocument::mutation_sources` map records which layer won
each id, so `mutation help <id>` can report it.

`src/application/document/mutations_loader/`. Native:
- App from `assets/mutations/application.json` via `include_str!`.
- User from `$XDG_CONFIG_HOME/mandala/mutations.json` (or
  `--mutations <path>` on the CLI).

WASM:
- App from the same embedded JSON.
- User from `?mutations=` query param or `localStorage`.

Map and inline are loaded from the document on every load. Each
layer is best-effort — user file parse failures log a warning
and skip; app bundle failures log an error (a build-time invariant
violation).

The provenance of each merged mutation is tracked in
`MindMapDocument::mutation_sources` as a `SourceTier`
(`src/application/source_tier.rs` — the same four-rung
`App` / `User` / `Map` / `Inline` ladder the macro registry
resolves against), so
`mutation help <id>` on the console can report which layer
won a given id.

### Declarative path — `MutatorNode` AST

A pure-data AST that compiles to a
`MutatorTree<GfxMutator>` and runs through the standard tree
walker.

Any mutation expressible as a tree of
field-level deltas with control-flow instructions belongs here.
This is the default: write JSON, the runtime walks it. The AST
shape mirrors `GfxMutator` — `Void`, `Single`, `Macro`,
`Instruction` — plus a `Repeat` wrapper for "N consecutive
children at consecutive channels with the same template" (the
flower-petal pattern, etc.).

`build_mutator(ast, context)` in `lib/baumhard/src/mutator_builder/`
walks the AST recursively, expands `Repeat` to N children with
incrementing channels, resolves `Runtime("<label>")` holes via the
`SectionContext`, and returns a fully-inflated `MutatorTree`. The
walker then applies it via `apply_to`. After application, changed
elements are synced back to the model so the change persists across
the next scene build.

### Imperative path — `DynamicMutationHandler`

A registered Rust function pointer the dispatcher
calls directly when the AST is too narrow.

Some operations are inherently imperative —
arbitrary BFS layouts, multi-pass spatial algorithms, anything
that needs runtime control flow the walker doesn't provide. The
handler registry lets them live as Rust functions registered at
startup, with the same `id`/contexts/target-scope surface as a
declarative mutation.

`src/application/document/mutations/`. Built-in handlers:
- `flower_layout.rs` — radial child arrangement.
- `tree_cascade.rs` — hierarchical cascading layout.

`register_builtin_handlers()` wires them at startup. Adding a new
handler is: new module, new function, new registration call, new
matching id in `assets/mutations/application.json`.

When a higher-precedence layer (User / Map / Inline)
declares the same id as a registered handler, **the declarative
mutator wins**. The handler is bypassed. This prevents a subtle
hijack where a user's JSON would silently invoke imperative code
they did not author.

### Target scopes

Seven variants telling the dispatcher which nodes the
mutation covers — also used as the snapshot window for undo.

A mutation declares "I touch this node only" or
"I touch this node and all its descendants" and the dispatcher
both walks the right subtree and snapshots the right set for
undo. The undo-snapshot equivalence is the load-bearing detail:
if a mutation's `target_scope` is too narrow, undo will not
fully reverse it.

Variants: `SelfOnly`, `Children`,
`Descendants` (not the anchor), `SelfAndDescendants`, `Parent`,
`Siblings` (the anchor's siblings, excluding itself — a root has
none, since "sibling" means "shares my parent" and the other
roots of a multi-root map do not), `SectionsOnly` (the anchor's
section-areas; the anchor `MindNode` is still the snapshot
window). Scope helpers in `custom_mutation::scope` produce
matching `MutatorNode` shapes for the AST walker.

The scope resolves to a target set and the mutator is anchored at
**each target in turn**, so a pairing has complete undo coverage
only when that set is closed under the mutator's reach. Only the
two descendant scopes are; every other scope needs a mutator that
touches its anchor alone. `TargetScope::covers_reach` encodes that
table and `warn!`s on a mismatch — see `format/mutations.md`.

Closure is about undo coverage alone, not about how often the
payload lands. Per-target anchoring runs a wider-than-`SelfOnly`
mutator once per ancestor target, so a `SelfAndDescendants` scope
paired with a `Descendants` reach writes a node at depth *k*
*k + 1* times — covered by the snapshot, but compounding for a
non-idempotent payload. The flat-apply path applies once per
target and so avoids this today.

### Behaviors — `Persistent` vs. `Toggle`

Whether the mutation commits to the model and pushes
an undo entry (`Persistent`) or only modifies the display tree
and remembers itself in `active_toggles` (`Toggle`).

Some mutations are "apply and remember"
(persistent — visual change, undo coverage); others are
"reversible inspection" (toggle — visual change without model
commit, second trigger reverses). Toggles are the right shape
for "highlight this", "expand this preview", "show debug
overlay".

Persistent: snapshot affected nodes, apply,
sync back, push undo. The sync-back
(`document/custom/sync.rs::sync_node_from_tree`) persists node
position, section offset / size / text / color+font runs, and
**font size** (the tree-side `scale` is distributed back across a
section's run `size_pt` values as a delta, preserving relative
run sizing); line-height is derived (`scale * 1.2`) so it needs no
separate home. Fields with no reverse converter (outline, shape,
zoom-visibility, line-height) are `warn!`-flagged at apply time
rather than silently applied-then-reverted. The undo entry and the
`dirty` flag are gated on the sync-back reporting an actual model
change, so a no-op apply (predicate filtered everything, non-flat
mutator skipped, or a mutation that changed nothing) leaves no
dead entry behind. Toggle: apply to tree only, insert
`(node_id, mutation_id)` into `MindMapDocument::active_toggles`;
on second trigger from the same anchor, remove the pair (undo
stack gets no entry — re-triggering is the reverse). Because a
rebuild projects the tree from the model and toggles never touch
the model, `build_tree` re-stamps every active toggle
(`reapply_active_toggles`) after each rebuild — without that
re-application a toggle-on's visual would die at the end of the
same dispatch and have nothing left to reverse.

### Contexts taxonomy

Dotted-namespace tags describing what a mutation
operates on: `"internal"`, `"map"`, `"map.node"`, `"map.tree"`,
plus the reserved `"plugin.<name>.<kind>"` namespace.

The console's `mutation list` filters by
context so users see only mutations relevant to their current
selection. `"internal"` hides a mutation from listing entirely
(used by handlers that compose into other mutations). The
plugin namespace is the home of future plugin-authored
mutations.

`matches_context(query)` returns true if the
mutation's `contexts` include `query` exactly or sit inside its
dotted prefix; `matches_context("map")` hits both `"map.node"`
and `"map.tree"`.

### `PlatformContext`

A three-variant enum — `Desktop`, `Web`, `Touch` —
threaded through mutation handlers and trigger-binding filters
so an authored mutation can branch on where it's running.

Some operations should behave differently per
target — a layout that reflows narrower on mobile, a trigger
binding that only fires on Desktop. `PlatformContext` is the
channel for that distinction, distinct from the dotted-namespace
"contexts" taxonomy above (which describes *what* the mutation
operates on).

Defined in
`lib/baumhard/src/mindmap/custom_mutation`. Today the variant is
chosen at compile time (`Desktop` on native, `Web` on WASM,
through `app::PLATFORM_CONTEXT`); the `Touch` variant exists but
no input path dispatches on it yet — including the tap path,
which reports the build's platform like every other input.
Relabeling a gesture would silently stop every `["Web"]`-gated
trigger from firing under a finger in the browser, so which
platform a build *is* stays a build fact until the authoring
format decides otherwise (`format/mutations.md`).
Embedded in `MutationApplicabilityGate.contexts:
Vec<PlatformContext>` so a mutation can declare which platforms
it applies to.

### Document actions

Canvas-level operations a mutation can carry
alongside (or instead of) its tree mutations:
`SetThemeVariant(name)`, `SetThemeVariables(map)`.

"Switch the theme" is not a per-node delta —
it touches `canvas.theme_variables`. Document actions cover that
seam. They run alongside the tree mutation; a single mutation
can both restyle nodes and switch the theme in one apply.

`lib/baumhard/src/mindmap/custom_mutation/document_action.rs`.
`SetThemeVariant` copies a named preset from
`canvas.theme_variants` into the live `theme_variables`;
`SetThemeVariables` overwrites individual entries while
preserving unmentioned keys.

### Animation timing

Optional duration / delay / easing wrapper around
any mutation, turning instant application into a clock-driven
interpolation.

A "grow font" that snaps is fine; one that
animates over 300ms reads better. The timing wrapper lets a
declarative mutation carry that timing without authoring an
animation by hand. The dispatcher starts an `AnimationInstance`
that ticks each frame, blends the in-flight state, and commits
on completion.

`lib/baumhard/src/mindmap/animation.rs` — reached as the `timing`
field on `CustomMutation`. Fields:
`duration_ms`, `delay_ms`, `easing` (`Linear` / `EaseIn` /
`EaseOut` / `EaseInOut`), and a reserved `then` (`Followup`)
slot. The app-side `AnimationInstance` that ticks it lives in
`src/application/document/types.rs`.

`Followup::{Reverse, Chain, Loop}` is named but not
yet wired. When it lands, mutations will compose into chains
and oscillations without scripting.

### Runtime holes — `SectionContext`

A trait the host implements to feed runtime values
into a `MutatorNode` AST at build time.

Some mutations need values the AST can't
inline — the count of currently-visible children, the cursor
position when invoked, a field looked up from the selected
node. `MutationSrc::Runtime("<label>")` and
`CountSrc::Runtime("<label>")` defer those holes to a
`SectionContext` registered per-mutation-id; the builder consults
it as it walks. Pure-data mutations (no holes) use a no-op
context.

`lib/baumhard/src/mutator_builder/context.rs`. The trait:
`fn count(&self, label) -> usize`,
`fn mutation(&self, label) -> Option<Mutation>`,
`fn area(&self, label, index) -> Option<GlyphArea>`. Custom
mutations register their context at apply time so the build
produces the right concrete tree.

---

## §5 The application runtime

The application runtime is the shell around the document. It owns
the event loop, the input state machines, the renderer, the
modal-UI state, and the keybind table. It does not own the data
model — that lives on [`MindMapDocument`](#mindmapdocument). The
split between "what changed" (document) and "what is on screen"
(renderer) is the model/view discipline at work.

Lives under [`src/application/`](./src/application/).

### `Application`, `InitState`, `NativeApp`

The native event-loop entry points. `Application` is
the pre-window root; `InitState` is the persistent post-window
state; `NativeApp` is the winit `ApplicationHandler` glue.

The platform separation is honest: pre-window
work (parse args, init fonts, load mutations) happens before any
GPU resources exist; once the OS gives us a window, we transition
to `InitState` and stay there for the lifetime of the run.
`NativeApp` exists only to satisfy winit's trait surface;
everything substantive lives on `InitState`.

`src/application/app/run_native.rs:48-130`.
`InitState` carries `window: Arc<Window>`, a
`document: Option<MindMapDocument>`, `drag_state`, `app_mode`,
modal UI state (console, node text editor, single-line editor,
color picker), `picker_hover`, and the resolved keybind table. The
`input_context()` method at line 137 produces a borrowed view of
these fields per-event so handlers can borrow disjoint subsets
without lifetime contortions.

The `Option` on `document` is the interactive shell's shape, not
startup's: init always produces a document, because a load that
fails produces the [load-failure placard](#load-failure-placard)
instead of nothing.

### Load-failure placard

The document a rejected map load is rendered as, so the loader's
message reaches the canvas instead of only `stderr`.

Startup is the surface most likely to be a user's first contact
with the format, and it used to be the only one that swallowed a
loader error — the console `open` verb puts the message in the
overlay, `maptool` puts it on `stderr` and exits nonzero, and
startup logged it and drew an empty canvas. Behind a double-click
there is no terminal, so that log line did not exist for the person
holding the file (#107).

A placard is an ordinary one-node `MindMap`
(`lib/baumhard/src/mindmap/placard.rs`) whose text is the headline,
the path or URL that was asked for, the loader's message in full,
and a line confirming nothing was written. Because it is a map, it
reaches the screen through the same tree projection as any other
document (§1 "Everything is glyphs") — no second pipeline, no
renderer pass, and nothing that needs a GPU to test. The message is
wrapped with `grapheme_chad::wrap_to_display_width`, so a path
containing an emoji folds without shattering.

"In full" has one bound, and it is not stylistic: a loader message
is as long as the file makes it, because serde quotes an offending
JSON string back verbatim. So the source and the message are each
elided in the *middle* — both ends kept, a one-line notice between
— past `PLACARD_HEAD_CLUSTERS + PLACARD_TAIL_CLUSTERS`. Nothing the
loader realistically emits comes near that (the longest, a 400-node
parent-cycle report, is 56 wrapped lines), and the budget is what
keeps the placard a map the loader itself accepts: unbounded, a
4 MB message asks for a node twice as tall as `MAX_NODE_AXIS`.

`src/application/app/startup_load.rs` is the decision. Both init
paths call `startup_load::adopt(startup_load::startup_surface(...))`
and nothing else: `StartupSurface` has one arm per outcome, `adopt`
is the only place the rejected arm is matched, and it is where the
message reaches the log. That is what keeps the browser build and
the desktop build reporting a bad map identically (§1
"Cross-platform as first-class"); the browser build's earlier DOM
overlay is gone, replaced by this. Adding a third outcome is a
compile error in `adopt` rather than a silent return to the empty
window.

The placard is bound to no file path, which is load-bearing rather
than incidental: `save` and `Ctrl+S` both refuse a document without
one, so a reflexive save cannot write the placard over the file
that failed to parse — a file that, being unparseable, is likely
the only copy of what its author was hand-editing.

The convention behind the choice is
[`CODE_CONVENTIONS.md`](./CODE_CONVENTIONS.md) §9: `expect` is for a
broken program precondition, and a user's malformed input is not
one.

### Event loop and `drain_frame`

The per-frame heartbeat: tick watchdog, drive
throttled interactions, advance animations, rebuild geometry,
rebuild scene if dirty, render, log frame interval.

Every frame runs the same six steps in the
same order. Inputs arriving between frames mutate the document;
the throttled-interaction shells and the per-frame geometry
flags ensure the next `drain_frame` rebuilds only what changed.
This decouples mutation frequency (often per-input-sample) from
rebuild frequency (at most once per frame), so a flurry of
pointer events doesn't trigger a flurry of scene rebuilds.

`src/application/app/drain_frame.rs`. Called
on every winit `AboutToWait` event. Step order:

1. Drive any active throttled interaction
   ([`ThrottledInteraction`](#throttledinteraction-and-throttleddrag)) —
   apply pending delta if the throttle says drain.
2. Advance running animations; on completion, push undo entry.
3. Rebuild connection geometry if edges moved.
4. Rebuild the scene where the frame's work requires it —
   mutation-path rebuilds run at their call sites
   (`rebuild_all`); the [dirty flag](#dirty-flag) is the
   unsaved-changes marker, not a rebuild trigger.
5. Dispatch to `Renderer::process` to push GPU buffers.
6. Update FPS rolling-average / snapshot counter.

### `MindMapDocument`

The data plane: owns the `MindMap`, the tree mirror,
the undo stack, the running animations, and the mutation
registries.

This is where every persistent piece of state
lives. It is the only owner of the model and the undo stack; the
renderer reads from it, never mutates. The dirty flag belongs to
it. Transient previews (live color picker, in-flight label edit,
in-flight portal-caption edit) belong to it too — read by the scene
builder, never committed back without an explicit step.

`src/application/document/mod.rs:64-151`. Fields include
`mindmap: MindMap`, `tree: Option<MindMapTree>`, `selection:
SelectionState`, `undo_stack: Vec<UndoAction>`,
`active_animations`, `active_toggles`, `mutation_registry`,
`mutation_handlers`, `mutation_sources`, `dirty: bool`,
`label_edit_preview`, `portal_text_edit_preview`,
`color_picker_preview`, `border_preview`,
`rect_select_preview` (the node ids a rubber-band rectangle
currently covers — see [`DragState`](#dragstate)). What it does
**not**
own: the renderer, GPU resources, drag/mode state, modal editor
state, keybinds — those are all on `InitState`.

### `SelectionState`

A tagged union of what the user has selected:
nothing, a node, multiple nodes, one section of one node, an
edge body, an edge label, a portal icon, or a portal text.

Selection variants are mutually exclusive by
construction — at most one thing is selected at a time. The
variant tag is the routing key for everything operating on the
selection: which clipboard channel a copy goes through, which
color field a color command sets, which font field a font
command sets. The renderer uses it to apply the cyan highlight
to the right element.

`src/application/document/types.rs`. Variants:

- `None`
- `Single(node_id)` — one node
- `Multi(Vec<node_id>)` — multiple nodes
- `Section(SectionSel)` — one [`MindSection`](#mindsection) of
  one node, identified by `(node_id, section_idx)`. Surfaces
  when the user clicks on a section-area in a *multi-section*
  node (single-section migrated nodes still route through
  `Single` so today's whole-node verbs keep firing on the whole
  node target). Per-section setters cover text
  (`set_section_text`, `set_section_text_and_runs`,
  `set_section_text_preserving_runs`), color
  (`set_section_text_color`), font (`set_section_font_size`,
  `set_section_font_family`), position + size
  (`set_section_offset`, `set_section_size`), the
  structured-clipboard payload (`apply_section_payload`), and —
  added in Batch 5 — structural mutators that change the
  `sections` vector length: `add_section` (insert with AABB
  validation against the parent), `delete_section` (remove
  with the "≥1 section per node" invariant enforced), and
  `split_section` (split text at a grapheme boundary; runs
  partitioned via `text_run_ops::slice`). The trait dispatcher
  and the `section …` console verbs route here from a
  `SelectionState::Section`, or from `Single(id)` on a
  single-section node (which §4.5 rule 3 auto-resolves to
  `(id, 0)`).
- `MultiSection(Vec<SectionSel>)` — two or more sections,
  possibly across distinct nodes. Built by shift+click on a
  section while another section (or section-set) is selected;
  each shift+click toggles the targeted section in / out of the
  set. Per-section verbs (color text, font size / family) fan
  out via `selection_targets` and apply to every section in the
  set. Per-section gestures (drag-to-move, drag-to-resize) stay
  single-target — a `MultiSection` selection emits no resize
  handles, and a press on a section in the set **demotes** the
  selection down to `Section(node, idx)` at threshold-cross so
  mid-drag picker hints + per-section verbs reflect the
  in-flight gesture's actual target rather than the prior
  multi-set. Whole-node move and node-resize gestures demote
  the same way (to `Single(node)`). The `section
  move` / `section resize` verbs target the single-section
  selected (or take an explicit `section=K` kv); `MultiSection`
  is fan-out-only at the trait dispatch layer.
- `SectionRange { sel, section_span, grapheme_range }` — one
  anchor section plus the two range meanings a section-scoped
  selection can carry, each in its own newtype so they can never
  share a slot again (#47 part C): `section_span: SectionSpan` is
  an inclusive span of **section indices** on the owning node
  (what border / style verbs fan out over, and what
  `cleanup_after_structural_mutation` clamps), and
  `grapheme_range: GraphemeRange` is a half-open sub-range of the
  anchor section's **grapheme clusters** (what the range-aware
  setters `set_section_text_color_range`,
  `set_section_font_size_range`, `set_section_font_family_range`
  and the range-aware clipboard cut / paste consume). Produced by
  the inline text editor's shift-select anchor on close: the
  editor lifts the (anchor, cursor) pair into `grapheme_range`
  with `section_span` covering just the anchor section. Accessors
  that only care about the owning section (`selected_section`,
  `is_selected`, `selected_ids`) treat it identically to
  `Section`. **Clipboard contract:** `Cut` and `Paste` are
  range-aware — cut removes the in-range graphemes and returns
  them as text; paste splices into the range; `Copy` falls
  through to whole-section copy because the structured payload's
  geometry belongs to the whole section. **Picker contract:**
  `ColorTarget::Section` and `PickerHandle::Section` carry the
  grapheme sub-range, so commit calls
  `set_section_text_color_range` directly (bypassing the
  `MultiSection` fan-out — different sections' lengths make
  cross-section sub-range semantics incoherent).
- `Edge(EdgeRef)` — the whole edge body
- `EdgeLabel(EdgeLabelSel)` — the text label of a line-mode edge
- `PortalLabel(PortalLabelSel)` — a portal endpoint icon
- `PortalText(PortalLabelSel)` — a portal endpoint text

The four edge-adjacent variants (`Edge`, `EdgeLabel`,
`PortalLabel`, `PortalText`) each route to a different
clipboard / color / font channel: copy on a `PortalLabel` reads
the icon color; copy on a `PortalText` reads the text color;
font commands write to the corresponding field group.

### `EdgeRef`

The `(from_id, to_id, edge_type)` triple that
identifies an edge.

Edges have no stable id (§3:
[`MindEdge`](#mindedge)), so selection, undo entries, and
console arguments all carry this triple. Equality and lookup are
by triple match against the model's `Vec<MindEdge>`.

`src/application/document/types.rs:71-97`. The `matches`
method walks the edge vector linearly; this is fine because
edges are sparse and the lookup happens at user-event frequency,
not in hot loops.

### `InteractionMode`

The single cross-platform interaction-mode enum that absorbed
the pre-redesign `AppMode` (Reparent / Connect) plus the new
Resize / NodeEdit modes the section-borders-resize PR added.
Five variants today: `Default` / `Reparent { sources }` /
`Connect { source }` / `NodeEdit { node_id }` / `Resize {
target }`.

Some user actions take two clicks (select a source, then click
a target); some put the canvas into a sub-context where chrome
and click-routing diverge (resize handles / per-section frames).
`InteractionMode` is the modal substrate for both shapes — what
the user is *doing right now*.

- `Default` — normal canvas navigation. Click selects, drag
  pans, edges snap.
- `Reparent { sources }` — the next left-click on a node
  attaches `sources` as its last children; left-click on empty
  canvas promotes them to root; Esc cancels. Triggered by
  Ctrl+R on a selection.
- `Connect { source }` — the next left-click on a target node
  creates a `cross_link` edge from `source`; left-click on
  empty canvas cancels. Esc also cancels. Triggered by Ctrl+D
  on one node.
- `Resize { target }` — chrome shows resize handles on the
  target (a `ResizeTarget::Node(id)` or
  `ResizeTarget::Section { node_id, section_idx }`). Drag a
  handle to resize; Esc returns to `Default`. Triggered by `r`
  keybind on a selectable AABB or by `mode resize`. Touch peer:
  `LongPress` — deliberately native-only, see
  `TouchGestureRecognizer` below.
- `NodeEdit { node_id }` — chrome dims sibling nodes and frames
  the active node's sections in cyan. Click a section to lift
  it into a `Section` selection; Enter (or `section edit`)
  opens the inline text editor on the active section. Esc /
  outside-click returns to `Default`. Triggered by `n` /
  `mode node-edit` / `node edit`.

`src/application/app/interaction_mode.rs`. The enum is cross-
platform (compiles + the field plumbs through `InitState` /
`WasmInputState`); several entry-point Actions
(`EnterResizeMode`, `EnterNodeEdit{,Clean}`, `EnterSectionEdit`,
`FastResizeStart`) are NativeOnly today because they depend on
the cursor-driven modal-stealer + DragState machinery that's
native-gated — the `LongPress` touch default dispatches the same
NativeOnly `EnterResizeMode` and so reports `Unhandled` on WASM,
where a one-shot warn names it rather than the gesture vanishing.
That one is a deliberate posture, not a pending port; the rest of
the touch vocabulary (tap, one-finger pan, pinch) reaches no
`Action` at all and works on both targets. See
`TouchGestureRecognizer` below.
Modal-stealer cascades route keystrokes per active mode (the
keybind resolver keys on `(InputContext, key)` and the modal
stealer can intercept e.g. Esc before normal dispatch).

The console verbs `mode resize` / `mode node-edit` /
`mode default` ride the same surface; `section edit
[section=<idx>]` and `node edit` are sugar over the
mode-flip + side-effect handler.

See `SECTIONS_BORDERS_RESIZE_PLAN.md` §2 for the design
problem this lifted, and §3-§4 for the resize / node-edit
mode UX. Plan §1 captured the three problems the
`InteractionMode` enum unified (the consolidation of the
pre-redesign `AppMode` into this enum is one of those).

### `DragState`

The drag state machine: `None` / `Pending` / `PendingRight` /
`Panning` / `SelectingRect` / `Throttled(ThrottledDrag)`.

Mouse-down does not commit to a drag yet —
the user might be clicking, or might be about to drag. `Pending`
captures everything the cursor was over at button-down
(`PendingRight` is its body-only right-button counterpart); once
movement crosses the drag threshold, the state transitions to
`Panning` (empty space), `SelectingRect` (Shift+drag on empty
space), or one of the seven `ThrottledDrag` variants depending
on what was hit.

`Pending` and `Throttled` carry boxed payloads, so `DragState`
itself is 64 bytes rather than the 912 the widest variant used to
impose on every state — including the `None` that is live for all
but a few seconds of a session. `PendingRight`, 64 bytes, is the
widest variant left and stays unboxed.

**`SelectingRect` splits its per-frame work in two, and only one
half runs every frame.** The overlay rectangle tracks the pointer
and is redrawn unconditionally. The covered-node preview is
memoized: `drain_rect_select` hit-tests against the *installed*
tree (a rubber-band drag mutates nothing, so its geometry is
current) and repaints only when `set_rect_select_preview` reports
the covered set actually moved. The set itself lives on
`MindMapDocument` beside the other transient previews, so every
rebuild path paints it — including the animation tick's, which
runs after this drain in the same frame and used to wipe a preview
stamped onto a tree the drain had built for itself. Before #37 the
drain ran `doc.build_tree()` plus a full text-buffer rebuild on
every frame of the gesture, under a comment calling it a
"lightweight overlay redraw".

**The variant is the authority; both artifacts are its
projection.** Because the covered set is document state that every
node-tree rebuild reads, a gesture that ended without it being
dropped leaves the whole canvas painting a rectangle the user let
go of — and `SelectingRect` has more ways out than the left-release
arm that used to be the only one dropping it (a left press
overwrote it with `Pending`, `Action::PanCanvas` with `Panning`,
and a target-picker mode swallowed the release entirely). So the
drain runs on *every* frame, not only the gesture's, and
re-derives the overlay rectangle and the covered set from the drag
state it finds; the two eager calls that remain (the release arm,
the target-picker mode entries) exist only so the rebuild they run
in the same breath does not paint the set one last time. Nothing
has to enumerate the exits, so a new one cannot strand it.

One consequence of hit-testing the installed tree is worth naming:
the drain runs *before* the animation tick in the same frame, so a
node an animation is advancing is hit-tested one tick behind where
it is drawn. The window is a single frame and only opens while an
animation and a rubber band are live at once; closing it would
mean either ordering the animation tick first (which would make
the rectangle lag the pointer) or going back to a per-frame arena
build.

`src/application/app/mod.rs`. Native-only today.

**Hit priority, in the three parts it actually has.** The line
that used to sit here read "edge handle > portal label > edge
label > node, so small grab-areas always win over larger AABBs",
and it was wrong in both clauses. What is true:

- **Capture, at button-down.** One chain runs, `compute_click_hit`,
  and it gives the *node* priority: a portal hit is resolved only
  when no node was hit, an edge-label hit only when neither was.
  `portal_label_drag_capture` then arms the portal drag on the
  same condition. So a marker or a label sitting over a node
  loses the press to the node — on the drag path and the click
  path alike, because there is one chain and the drag reads its
  answer. A larger AABB in front wins.
- **Capture, outside that chain.** The press also runs three
  handle hit-tests that never consult `hit_node`, each gated on
  state instead: `hit_edge_handle` on the selection being
  `SelectionState::Edge`, `hit_node_resize_handle` on
  `InteractionMode::Resize { Node }`, and
  `hit_section_resize_handle` on `Resize { Section }`. Handles
  are what the mode or the selection *is for*, so a press near
  one is read as aimed at it whatever the chain says.
- **Promotion, at threshold-cross.** `event_cursor_moved.rs`
  consumes the captured hits in the order edge-label →
  portal-label → edge-handle → node-resize-handle →
  section-resize-handle → section-move → node-move →
  Shift-rect-select → pan. Over the label hits this order decides
  nothing, since the capture rule above leaves at most one of them
  and the node populated. What it does decide is **all three
  handle families versus the node**: each is ranked ahead of
  `hit_node`, and each was captured without asking about it.

**Where a click and a drag on the same press name different
things** follows from that: the click ladder has no rung for
handles at all, so every hit in the second bullet is a target the
drag path can reach and the click path cannot. Three ways in, one
shape.

It is reachable, not theoretical. `resize_handle_positions` puts
the eight handles *on* the boundary of the box they resize and
`nearest_handle_within` accepts anything inside
`HANDLE_HIT_TOLERANCE_PX` of one, while `hit_test_target` returns
`None` for a point `point_in_node_aabb` refuses — and that
predicate is shape-aware. So a press a few pixels outside a
corner, or exactly on the corner handle of an ellipse-shaped node
(where the corner is outside the shape by construction), is empty
canvas to the click ladder and a resize to the drag: hold still
and the selection clears, move and the node resizes. Where the
handle sits *over* a node instead, the same press selects that
node on a click and drags the handle on a drag.

That is the intended shape — a handle is a drag affordance, and a
click through it falling back to selection is what makes the
handle non-modal — not a divergence to repair.

Issue #48 reported the *portal* case as such a divergence,
reading the promotion order without the capture rule.
`test_portal_label_drag_capture_is_gated_by_the_same_node_hit_the_click_ladder_uses`
now holds the two paths together, and names the gate that had
been an unpinnable inline `if` inside the press handler.

**A press never clobbers a gesture already in flight.** All three
press paths in `event_mouse_click.rs` check the drag state before
writing it, at two widths:

- **Right and middle** refuse whenever the state is anything but
  `None` (`handle_right_button`'s inline guard;
  `route_middle_button`'s `Keep`). Neither button has a re-arming
  role to preserve — a right press arms `PendingRight` and a
  middle press dispatches `MouseGesture::MiddleClick`, and both
  are meaningful only from rest.
- **Left** refuses on the narrower
  `DragState::would_abandon_gesture` class — `PendingRight` and
  `Throttled(..)` — because it *is* the arming press for
  `Pending`, and a click after a release the window never
  delivered has to re-arm rather than do nothing.

The class is the point: `Throttled(..)` owes the model a write
and an undo entry that only `commit_on_release_core` performs,
so replacing it leaves the tree holding dragged offsets that the
next model rebuild silently snaps back — position loss with
nothing to undo. `Panning`, `SelectingRect` and `Pending` owe the
model nothing and are re-derived from the next sample or press.

**The same class guards the *Action*, not only the presses.**
`Action::PanCanvas` is the one Action that writes a drag state
from rest, and it has three entry points, not one:
`MouseGesture::MiddleClick`, `MouseGesture::LeftDrag` (which only
ever runs from `Pending`), and any keyboard binding or macro step
naming `pan_canvas` — a first-class user-bindable entry that
`event_keyboard` dispatches with no drag-state check and that
`SourceTier::allows_action` does not gate. So the guard sits on
the arm (`dispatch::native::route_pan_canvas`, pure and pinned at
`DragState` level the way `route_middle_button` is), which closes
all three at once. `Pending` stays outside the refused class
precisely because the `LeftDrag` threshold cross dispatches
`PanCanvas` from it.

The release halves match, and the rule is the same one: **the
button that started a gesture is the one that finalizes it.**
`resolve_release` commits only on the owning button and answers
`PutBack` for every other, which both dispatchers honor by
restoring the drag state. A stray right-click cannot end a
left-button drag, and — since the left press started being
refused mid-gesture — a left release cannot end a right-started
fast-resize either; the right release, which the user still owes
because the button is down, commits it. Middle-click was the
exception on both halves until #37 — it overwrote any state on
press and forced `None` on release — and the right-button guard's
comment named that overwrite as the posture it was rejecting.

**One state is outside the rule, and knowingly.** `Panning` does
not record which button armed it, and three things arm it (a
middle press, the `LeftDrag` threshold cross, any keyboard or
macro `pan_canvas`), so a middle release during a left-drag pan
ends it and a left release ends a middle-started one. Nothing is
at risk — `Panning` owes the model no write and no undo entry,
which is exactly what puts it outside the class above — and
closing it is a decision about `Action::PanCanvas`'s semantics
rather than a guard: the variant would have to carry its origin,
and a keyboard-armed pan has no button to name, so "momentary or
modal" has to be answered first.

### `TouchGestureRecognizer`

The touch half of the pointer vocabulary, and `DragState`'s
peer rather than its consumer: a plain-value state machine
that turns raw `(phase, finger_id, position, now)` tuples into
a typed `RecognizedGesture`.

A finger is not a mouse, and winit's mouse synthesis
does not cover a hold, a second finger, or a drag that never
pressed a button. Without a recognizer the browser — the
surface `run_wasm/mod.rs` calls the *primary* one this
project targets — has no input path at all on a phone.
`Idle ↔ OneFinger ↔ TwoFingers` with four emit points:

- **`Tap`** on the lift of a finger that never left
  `POINTER_DRAG_THRESHOLD_PX` and was down for less than
  `LONG_PRESS_MS`.
- **`LongPress`** from `tick(now)` — "held for 350 ms" is a
  wall-clock transition, not an event — once per episode.
- **`Pan { pos, delta }`** on every `Moved` from the threshold
  crossing onward. The first emission carries the travel since
  the finger *landed*, so summing an episode's deltas gives
  the finger's net displacement and the threshold's slop is not
  lost.
- **`PinchStep { center, pan, scale }`** while two fingers are
  down, whenever their midpoint or their separation has moved
  past the threshold since the last emission. Both halves ride
  every emission, because two fingers moving describe a
  translation and a scale at once; a parallel two-finger drag
  therefore reports steps whose `scale` multiplies to 1.0, and
  a symmetric spread steps whose `pan` sums to zero.

**No clock is read and no I/O happens inside it** — time is a
parameter — which is what makes every rule above provable on a
machine with no touchscreen (`TEST_CONVENTIONS §T9`). What
cannot be proved that way is the layer below: that a real
digitizer delivers the phases in the order the machine assumes.

**Two routes out, and the split is the point.** `LongPress`
resolves through `MouseGesture::LongPress` and the keybind
table, exactly as a mouse gesture does — it is the only touch
gesture that reaches the table. The other three carry no
`Action` at all: they take the two carve-outs CODE_CONVENTIONS
§3 already grants the mouse, the pre-funnel selection
bookkeeping a single click runs and the per-frame camera delta
a drag runs, through one cross-platform
`dispatch::apply_touch_effect`. That is why tap, pan and pinch
cannot be dead on one target: an `Action` can be `NativeOnly`,
and a `RenderDecree` cannot.

`Action::PanCanvas` is *not* on that path and could not have
been. It does not move the camera — it arms `DragState::Panning`
(`dispatch::native`'s `route_pan_canvas`), which is native-only,
so dispatching it from the browser returns `Unhandled` and warns.

`src/application/app/touch_gesture.rs` for the machine;
`dispatch/cross_dispatch/pointer.rs` for `drive_touch_event`
(recognize → route) and `apply_touch_effect` (run). Both
runtimes call both: `run_native.rs::dispatch_touch_event` and
`run_wasm/event_touch.rs`.

**What is native-only, and what that costs.** `LongPress`
ships bound to `EnterResizeMode`, which is `NativeOnly`, so on
the browser it dispatches, returns `Unhandled` and fires a
one-shot warn naming the remedy. That is deliberate rather than
pending: long-press is the touch peer of the keyboard's `r`,
and rebinding the browser's long-press to some unrelated
Compatible Action would make one gesture mean two different
things on two targets — a worse §4 outcome than a gesture that
is honestly unavailable and says so. Its parity rides on
`InteractionMode::Resize`'s chrome and handle-drag reaching the
browser. `TwoFingerDrag` used to sit beside it, bound to
`FastResizeStart` and equally dead there; two fingers now drive
the camera instead, and the variant is gone rather than
default-unbound, because a binding firing on the same event as
the camera step would be a conflict rather than a choice.

Two consequences of that deletion, neither fixed: a user whose
only pointer is a touchscreen loses `FastResizeStart` (it ships
bound to `Ctrl+RightDrag` alone, which needs a mouse), and an
existing `keybinds.json` naming `"TwoFingerDrag"` is silently
accepted and never matches, because `KeyBind::parse` takes any
non-modifier word and cannot tell a retired gesture name from an
ordinary key. `test_two_finger_drag_is_not_a_bindable_gesture`
carries the reasoning.

### `ThrottledInteraction` and `ThrottledDrag`

A trait pair + seven-variant enum providing one uniform
shell for the whole lifecycle of a continuous, high-rate-input
drag.

Dragging a node, a section, a section's
resize handle, a node's resize handle, an edge handle, a portal
label, and an edge label all follow the same three-phase pattern:
fold each cursor sample into pending state, ask the throttle
whether to drain and apply if so, and commit to the model when
the button comes up. All three phases live here; new throttled
drags attach as one struct + one trait impl + one enum variant
without growing either event-file dispatcher.

`src/application/app/throttled_interaction/mod.rs`.

`ThrottledInteraction` is the drain shell: `pending()` /
`pending_mut()` are the only required state accessors, and
`has_pending`, `throttle`, `should_perform_drain`,
`needs_continuation` and the `drive` shell are all provided from
them. Implementors add a `drain(ctx)` body and, rarely, a `reset`.

`ThrottledDragInteraction` adds the two phases a drag has and the
picker-hover interaction does not: `accumulate(DragInput)`
(provided) and `commit_on_release_core(ReleaseCommit) ->
ReleaseRefresh` (required). The core is required so that a new
variant cannot compile with no release behavior at all, and it is
the *only* release entry point on the trait — `ReleaseCommit`
carries no renderer, so a commit body has nothing to reach one
with. Running the decree needs `&mut Renderer` and lives on
`ReleaseRefresh::execute`, off the gesture trait entirely. What
stays convention is the drain half: `drain` and its `drive` shell
take a `DrainContext` because a per-frame drain genuinely
repaints.

`ThrottledPending` (`throttled_interaction/pending.rs`) owns the
pending half. Three disciplines cover every implementor, and each
picks one at construction:

- **delta-accumulate** — the drain applies an incremental
  movement, so skipped samples sum;
- **cursor-latch** — the drain projects an absolute position, so
  only the last sample carries information;
- **dirty flags** — nothing accumulates; a flag says the next
  drain has work (the picker-hover interaction).

`DragInput` carries one cursor sample in *both* forms — the
canvas-space delta since the previous event and the absolute
canvas-space position now — so the dispatcher never has to know
which discipline the active gesture uses.

`ReleaseRefresh` (`throttled_interaction/release.rs`) is the
canvas work a commit owes once its model write has landed:
`None`, `SceneOnly`, or `All`. Named rather than performed, so
the commit body stays renderer-free.

Variants:

- `MovingNode(MovingNodeInteraction)`
- `MovingSection(MovingSectionInteraction)` — drags one section's
  `offset` relative to its owning node; threshold-cross promotes
  here when the press lands on a section of a multi-section node.
- `SectionResize(SectionResizeInteraction)` — drags one resize
  handle of a `Some`-sized selected section. Threshold-cross
  promotes here when the press lands on one of the 8 handles
  (corners + edge midpoints); release commits a single
  `(offset, size)` write through `set_section_aabb`.
- `NodeResize(NodeResizeInteraction)` — drags one resize handle
  of a `Single`-selected node. Threshold-cross promotes here
  when the press lands on one of the node's 8 handles; release
  commits a single `(position, size)` write through
  `set_node_aabb`.
- `EdgeHandle(EdgeHandleInteraction)`
- `PortalLabel(PortalLabelInteraction)`
- `EdgeLabel(EdgeLabelInteraction)`

`as_dyn_mut()` / `as_dyn()` widen to
`&mut dyn ThrottledDragInteraction` — the drag trait, which has
`ThrottledInteraction` as a supertrait, so one ladder serves all
three phases. Those two matches are the only per-variant matches
in the crate: `event_cursor_moved`'s accumulate arm,
`event_mouse_click`'s left-release arm and its right-release arm
are each a single call through them.

Touch does **not** ride this ladder. Pinch-zoom, one-finger pan
and tap-select ship as `TouchGestureRecognizer` emissions that
reach the camera and the selection directly, because the
recognizer already is the state machine `ThrottledDrag` would
have supplied and the camera work is `CODE_CONVENTIONS §3`'s
continuous-gesture carve-out. What would earn a variant here is
a touch gesture that *mutates the document* per frame — dragging
a node with a finger — and that wants the browser to have a
drag-state machine first.

### `MutationFrequencyThrottle` (and `frame_throttle`)

An adaptive frame-counter throttle that gates
*application* of mutations under load while leaving *acceptance*
of input untouched.

When per-frame work threatens the GPU
budget, the system must degrade gracefully. The non-negotiable
rule is **responsiveness is never traded for fidelity**: the
cursor must stay current with the hardware pointer at all
times, even if the dragged node updates only every fourth
frame. The throttle samples actual work duration into a moving
average; if the average exceeds budget, it raises `n` (the
"drain divisor"); if work is well under budget with hysteresis
margin, it lowers `n` toward 1.

`src/application/frame_throttle.rs:64-183`.
Default budget `14_000` µs (60 Hz minus safety), default
window 8 frames, default hysteresis 30%. `n` clamps in
`[1, 8]`. Each `ThrottledDrag` owns its own throttle, so
per-gesture profiles tune independently — a 500-node move
budget does not bias an edge-label drag's average.

### `UndoAction`

A 13-variant tagged union; one variant per
user-facing mutation, dispatched through `MindMapDocument::undo`
to reverse it.

Every persistent change pushes one
`UndoAction`; Ctrl+Z pops the back of the stack and dispatches.
The discipline is **one mutation, one variant** — adding a new
mutation means adding a new variant, snapshotting the right
"before" state, and writing the matching `undo()` arm in the
same commit.

`src/application/document/undo_action.rs`. The thirteen
variants: `MoveNodes`, `CustomMutation`, `ReparentNodes`,
`DeleteEdge`, `CreateEdge`, `EditEdge`, `CreateNode`,
`EditNodeText`, `EditNodeStyle`, `EditNodeZoom`,
`CanvasSnapshot`, `EditNodeAabb`, `DeleteNode`. `CustomMutation`
is the general bucket — it snapshots the `target_scope`-defined
window so any declarative or imperative mutation replays
cleanly. Every arm is bounds-checked (e.g. `index <
edges.len()`) before mutating, so undo is always safe — never
panics, even on a partially-deleted state.

### The node-edit envelope and `NodeEditTail`

The one place the *push* side of an `UndoAction` is written for
node-scoped edits: snapshot → closure verdict → undo-push →
auto-fit.

`UndoAction` says what a reversal looks like; the envelope says
how a setter records one. Three of the variants describe a
per-node edit — `EditNodeStyle`, `EditNodeText`, `EditNodeAabb`
— and each used to be open-coded at every setter that produced
it, together with the `grow_one_node_to_fit_text` /
`grow_one_node_to_fit_border` tail. That fan-out drifted into
shipped bugs twice: a copy was corrected, its siblings were not.

`src/application/document/nodes/undo_envelope.rs` holds one
implementation. `mutate_node_with_style_undo` and
`mutate_node_with_text_undo` take a closure returning
`Some(value)` to commit or `None` to declare a no-op — on `None`
the envelope restores exactly the fields the undo entry would
have restored and pushes nothing, which is what lets a caller
mutate speculatively instead of reaching for the
`undo_stack.pop()` anti-pattern. `mutate_node_with_aabb_undo`
computes its own verdict by comparing `(position, size)`
**after** the auto-fit tail, which is what makes repeated writes
idempotent on a framed node whose border-grow overshoots the
requested size. Two section-scoped wrappers narrow the closure
to one `MindSection` and fold the index lookup in, so a stale
index is a no-op rather than a panic.

`NodeEditTail` is the fourth argument and the named policy for
what runs after a commit: `None` (color-only edits, which must
not re-measure), `Border` (the explicit-shrink and
border-config paths), `Grow` (anything that can change measured
text extent), `GrowAndCleanup` (the structural mutators — the
only edits that can strand a selection or a border preview on a
dead section index). Naming it is the point: an auto-fit pass
that is a copied suffix is a pass nobody chose.

The edge side has the same shape one layer over:
`MindMapDocument::mutate_edge` is the single `EditEdge`
envelope, and `edges/font_triple.rs` holds the one
`(size, min, max)` resolution — request ordering, inverted-bounds
guard, clamp — that the body, label, and portal-text font
channels share.

### `Renderer`

The GPU resource holder and command-buffer builder;
reads from the document, writes to the swapchain.

The `Renderer` is the view side of the
model/view split. It owns wgpu device, queue, surface,
pipelines, atlases, and the FPS ring buffer; every frame, its
`process()` reads document and scene state, builds command
buffers, and submits to the GPU. It never holds a reference to
the document.

`src/application/renderer/mod.rs:224-878`.
The dual pipeline lives here:
- **Rect / SDF pipeline** — node fills, ellipse SDF (shape-aware
  fills via `RECT_SHADER_WGSL`), background fills.
- **Glyph pipeline** — every visible character, via
  cosmic-text + glyphon atlas.

Sub-passes sit in `borders.rs`, `connections.rs`,
`console_pass.rs`, `color_picker.rs`. Visibility culling
combines `Camera2D::is_visible` (spatial) with
[`ZoomVisibility`](#zoomvisibility) (window).

### `AppScene` and scene host

A two-role scene container: a camera-transformed
canvas and a screen-space overlay, each composed of named
sub-trees.

Mindmap content (nodes, borders, connections,
portals, edge handles) belongs in the canvas role — pans and
zooms with the camera. The console and color picker belong in
the overlay role — fixed in screen space. The `AppScene`
abstracts that split; rebuild dispatch
(`InPlaceMutator` for small mutator-able changes,
`FullRebuild` for structural changes) flows through the same
seam for both roles. Canvas roles that also record a *content*
signature ask both questions through one entry point,
`canvas_dispatch_with_content`, which returns a third outcome —
do nothing, because the registered tree is already what a
rebuild would produce. One function rather than two calls per
role because the *order* is the design: structure first, content
second, and swapping them still renders correctly while undoing
the whole point (see
[canvas-role projection](#canvas-role-projection)).

`src/application/scene_host.rs:1-150`. Each
role has named slots (`CanvasRole`, `OverlayRole`); each slot
has a corresponding `Tree<GfxElement, GfxMutator>` and a
mutator registry. The same idiom drives both canvas-role
rebuilds (in `scene_rebuild.rs`) and overlay-role rebuilds
(console text changes, color picker re-layout).

### Scene shape cache

The rule that decides, per walked element, whether
its cosmic-text buffers and background fill are re-shaped or
reused — one mechanism serving both of
[`AppScene`](#appscene-and-scene-host)'s sub-scenes.

Rebuild dispatch stops at the arena. A mutator
apply reaches every slot of a role by design, and every writer
above it assigns every field whether or not the value moved, so
"mutate, don't rebuild" bought a reused arena and then paid for a
full re-shape of it anyway: a picker hover shaped all 59 overlay
elements, a console keystroke all ~50 rows, and every one of the
ten `flush_canvas_scene_buffers` call sites shaped every buffer of
all eight canvas roles no matter which one it had re-projected.

`src/application/renderer/scene_shape_cache.rs`.
`refresh(scene, ids, shaped, kind)` walks the sub-scene in layer
order and keeps, per walk position, a `ShapedSceneElement` holding
the output *and a verbatim copy of every input it was shaped from*
— tree id, `unique_id`, registered offset, and the `GlyphArea`.
Nothing announces a change: the pass re-validates against the live
tree, which is why a new writer of scene-tree state cannot make it
stale by forgetting to notify anyone, and why the ten canvas call
sites are answered as a class rather than one at a time.

Two details are load-bearing. `GlyphArea`'s derived equality is
blind to a region *recolor* — `ColorFontRegion`'s `Eq` is set
identity by range — which is exactly what a picker hover changes,
so the check adds `ColorFontRegions::same_content` on top of `==`.
And each element's background fill lives inside that element's own
entry rather than in a flat list, so a partial re-shape cannot
disturb the index order the rect pipeline paints in; the mindmap's
own keyed re-shape, whose fills *are* flat, needs
`BackgroundRectSlot` to hold that line by bookkeeping instead
(`renderer/tree_buffers.rs`).

`ScenePassKind` is the whole difference between the two passes:
the canvas keeps the walker's fills because they reach a draw
pass, the overlay discards them because no screen-space rect
pipeline exists yet. `refresh` takes a `&Scene` rather than a
`&mut Renderer` so its granularity can be asserted without a live
device (TEST_CONVENTIONS §T8) — the same reason
[`RebuildTier`](#scene-rebuild-granularity) is a value rather than
a statement.

### Scene rebuild granularity

Five tiered rebuild functions, each scoped to a
specific change kind.

Different changes invalidate different
amounts of work. Editing a node's text might change its width
(full rebuild); dragging a node only moves connection paths
(connection-only rebuild); changing a portal endpoint color
only touches portal markers (portal-only rebuild). Each tier
is dispatched explicitly so the cheapest one runs.

`src/application/app/scene_rebuild.rs`.
Functions: `rebuild_all` (node tree + every canvas role),
`rebuild_scene_only` (reuse the node tree, refresh every canvas
role), `rebuild_selection_highlight` (node text buffers only — no
canvas roles, no mode-status line: what a change to *which nodes
are highlighted* actually needs), and the per-role methods on
[`CanvasFrame`](#canvas-role-projection) —
`update_connection_trees` (edges + their grab handles),
`update_portal_tree`, `update_border_tree`,
`update_connection_label_tree`, `update_section_frame_tree`, and
the two resize-handle updaters — each callable on its own so a
caller refreshes only what its interaction can change.

**`RebuildTier` names the top two rather than running them.**
`RebuildTier::{All, SceneOnly}` is the choice between the first
two as a value, with `execute` as the only place either is
actually called. The reason is the same one
[`ReleaseRefresh`](#throttledinteraction-and-throttleddrag) has on
the drag-release path: both tiers need `&mut Renderer`, so an
interaction that *decides* a tier could not be tested at all
while deciding and performing were one statement (TEST_CONVENTIONS
§T8 keeps live wgpu out of the harness). Handing the decision back
as a value is what lets a test ask which tier a given interaction
picks.

Two constructors carry the rules:

- `for_selection_change(prev, new)` — `All` when either side is a
  node-ish selection (`Single` / `Multi` / `Section` /
  `MultiSection` / `SectionRange`), because section-area
  highlights are stamped into the node tree's
  `ColorFontRegions`; `SceneOnly` when both are edge-adjacent and
  only the scene-level cascade moves.
  `rebuild_after_selection_change` is this constructor plus
  `execute`, and stays the one-liner for callers with nothing
  else to weigh.
- `for_click(triggers_fired, prev, new)` — the same, except a
  fired `OnClick` trigger forces `All`, since a trigger's
  document actions are unbounded (a theme switch repaints every
  node). Both targets read this one function: native used to run
  `rebuild_all` for every click outcome, and the browser used to
  run the selection-delta tier even when a trigger had just
  mutated the document, so §4's peers were wrong in opposite
  directions.

`highlight_entries_for(doc)` is the one mapping all four
node-tree rebuild sites use for *which* nodes to tint: the
rubber-band preview when one is live, `doc.selection` otherwise.
The preview replaces rather than adds to the selection's entries,
because the gesture's release writes `SelectionState::from_ids`
over the selection outright — painting both would show a set that
is about to be discarded.

Interactions that decide a tier are split into a renderer-free
core that returns one and a shell that runs it —
`click::handle_click_core` / `click::handle_click`,
`event_cursor_moved::arm_label_drag` /
`event_cursor_moved::start_label_drag`. Both label promotions
(edge-label and portal-label) go through the second pair, so they
cannot pick different tiers again.

### Dirty flag

A single `bool` on `MindMapDocument` marking
unsaved changes: set by every document setter, cleared at
construction and on a successful `save`.

The user must not silently lose work. The
flag is the "there are changes worth saving" bit guarding
destructive document swaps.

`src/application/document/mod.rs` (the `dirty`
field). Set by the setter families under
`document/{nodes,edges}/`; cleared at construction
(`document/mod.rs`), by the `save` console verb
(`console/commands/save.rs`), and by the Ctrl+S save
(`save_document_to_bound_path`, `app/console_input/exec.rs`);
read by the `open` and `new` verbs' guards, which refuse to
replace a dirty document ("unsaved changes; save before…").
Despite the name it is **not** a render or rebuild signal —
scene rebuilds are call-site-driven (`rebuild_all` after
mutations), and the per-frame drain consults the renderer's
separate `connection_geometry_dirty` flag instead
(`app/drain_frame.rs`).

### FPS overlay

Two display modes for frame-time diagnostics:
snapshot (stable readout, re-sampled periodically) and debug
(live rolling average).

Performance-conscious development needs a
truthful FPS readout. The snapshot mode answers "what is the
steady-state frame rate?"; the debug mode answers "where are
the hitches?". The `fps` console verb (native) toggles between
them.

Embedded in
`src/application/renderer/mod.rs`. Both modes read
**wall-clock** deltas via `Instant::now()` stored in
`Renderer::last_frame_instant` — measuring render-body time
would lie under stress, because `render()` early-returns on
font-system lock contention and would collapse the reported
frame cost to near zero. The render-side plumbing
(`fps_display_mode`, `fps_overlay_buffers`, `set_fps_display`,
`tick_fps`, `RenderDecree::SetFpsDisplay`) compiles on both
targets; only the `fps` console verb is native-gated because
the console itself is. Browsers expose FPS via DevTools so the
WASM parity gap is cheap to leave.

### `FreezeWatchdog`

A native-only background thread that reads an
atomic timestamp pinged by the main loop and aborts the
process with a diagnostic banner if the main loop stalls past
threshold.

Mandala is single-threaded; an infinite
loop, a same-thread `RwLock` re-entry, or a blocking GPU call
would hang indefinitely with no actionable error. The watchdog
turns a hang into a fast, diagnostic crash. It is the only
background thread of a default launch — CODE_CONVENTIONS §3
additionally sanctions the [IPC boundary
threads](#ipc-server-ipcserver), which exist only under `--ipc` —
and the single-threaded invariant for the model/view pipeline is
preserved because the watchdog only *reads* a shared `AtomicU64`,
never touching app state.

`src/application/app/freeze_watchdog.rs:38-134`. Main thread
calls `tick()` at every event-loop boundary; watchdog reads
the atomic every second; if the gap exceeds `FREEZE_THRESHOLD`
(10 seconds), prints diagnostics and aborts. Not present on
WASM — browsers already provide an "unresponsive tab" dialog
for free.

### IPC server (`IpcServer`)

The opt-in local socket an external process — an LLM agent, a test
harness, `mandalactl` — drives the running app through, speaking
newline-delimited JSON (`format/ipc.md`).

The app's feedback loop for an agent: attach to an instance a human
launched (or one the agent launched itself), send commands, read
structured replies. Off unless `--ipc <path>` is passed; without the
flag no socket exists and no thread runs.

`src/application/ipc/`. Boundary threads own the socket — a
persistent acceptor (one controller at a time; a second client is
refused immediately), plus a reader and an outbound writer per
controller — and exchange only protocol values with the main thread
over one ordered `mpsc` queue in and a bounded one out. They wake the
parked event loop through a winit user event (`IpcWake`), so an idle
app answers without polling; `about_to_wait` drains the queue right
after `drain_inputs` and executes every command on the main thread,
with the same access an input handler has. Each connection
carries a generation stamp, so nothing a departed controller sent can
act after it left. A controller that falls behind on its replies is
paused rather than dropped; one whose socket accepts nothing for ten
seconds is dropped. The socket is created owner-only, refuses any
directory on its path that another user could rename it out of, and
replaces an existing path only when it is provably a stale socket of
ours.

Commands live in a registry (`ipc/registry.rs`), one module per family
under `ipc/commands/`, and `describe` renders that same table, so the
discovery surface cannot drift from the dispatch surface. Each command
runs inside a log-capture window (`baumhard::util::log::capture`), so
every `warn!` / `error!` it causes also reaches its reply's
`warnings`. The `act` family is the dispatch bridge: `act.action`,
`act.console` and `act.macro` reach `dispatch_action`, the console's
`execute_console_line` and the macro dispatcher's gated step loop
through `CommandTarget`, the app-side execution context. There is no
parallel control path, so undo, gates and side effects match the
keystroke each stands in for.

**Vision:** the families after `act` (query, scene, input, capture,
clock, events) each add one module and one line to the registry's
family table.
**Caveat:** Unix-only today — Windows fails `--ipc` at startup until
the named-pipe adapter lands, and the browser has no socket at all
(`CLAUDE.md` "Dual-target status").

### `now_ms()`

A cross-platform monotonic clock returning `f64`
milliseconds since process start (native) or page load (WASM).

Animation timing, double-click detection,
FPS tracking, throttled-interaction frame stamping all need a
clock that works the same on both targets. `now_ms()` is the
single bridge.

`src/application/app/mod.rs:98-111`.
Native: `Instant::now()` deltas from a static epoch. WASM:
`window.performance.now()` (clamped to ≥1ms by Spectre
mitigations).

---

### Action dispatch

Every user-driven application-level effect is a variant
of `enum Action` (`src/application/keybinds/action.rs`) and runs
through a single `dispatch_action(action, ctx, hit)` funnel
(`src/application/app/dispatch/native.rs`). Mouse, keyboard, the future
macro runtime, and any plugin host all reach the same arms.

Before this funnel existed, mouse gestures
(double-click create-orphan, double-click open-editor, middle-click
pan, wheel zoom) were hardcoded inside event handlers and bypassed
the keybind system entirely. Users couldn't disable, rebind, or
replace them without recompiling. The funnel reifies every gesture
as an Action so one vocabulary covers keys, mouse, macros, and
plugins.

`KeyBind` (`src/application/keybinds/bind.rs`)
accepts mouse-shaped binding strings —`DoubleClick`, `MiddleClick`,
`RightClick`, `LeftClick`, `LeftDrag`, `WheelUp`, `WheelDown` —
alongside keyboard names. Mouse handlers synthesize the gesture's
canonical name via `gesture_key_name(MouseGesture::*)` and feed it
through the same `ResolvedKeybinds::action_for_context` lookup as
keyboard input. Lookup → `Action` → `dispatch_action(action, ctx,
Some(&DispatchHit { click_hit, canvas_pos }))`.

**Resolution order** (any binding):
1. `keybinds.action_for_context(...)` — built-in `Action` variants.
2. `keybinds.macro_for(...)` — user-defined macros, loaded from
   `~/.config/mandala/macros.json` on native. See
   `crate::application::macros` for `Macro`, `MacroStep`,
   `MacroRegistry`, and `dispatch_macro`. Steps fan out to
   `dispatch_action`, `apply_keybind_custom_mutation`, or
   `execute_console_line` depending on `MacroStep` kind, so plugin
   authors and macro recorders share one runtime path. That path is
   `run_macro_steps`, the gated step loop `dispatch_macro` calls
   after its lookup. IPC's `act.macro` runs an anonymous step list
   through it too, so there is one privilege gate however a macro
   arrives. **Unknown
   macro id falls through** to the custom-mutation tier, so a
   typo'd or half-loaded macros file doesn't swallow the keystroke.
3. `keybinds.custom_mutation_for(...)` — per-node custom mutations.

**Built-in Actions win on collision.** A key combo bound to both
`Action::Copy` (in `copy: ["Ctrl+C"]`) and a macro on `"Ctrl+C"` in
`macro_bindings` runs the Action — the macro never gets a chance.
To override a built-in Action with a macro, first unbind the
Action's keybind (set `copy: []`) and then bind the macro. Same
applies for built-in vs. custom-mutation collision.

**Macro privilege model.** Macros are tagged by their loader tier
(`SourceTier { App | User | Map | Inline }`); the dispatcher
fail-closes on tier-restricted surfaces (`ConsoleLine` and
destructive / I/O `Action` variants). Authoritative threat model
+ surface enumeration + tier-by-tier permissions:
[`format/macros.md`](./format/macros.md). The `#[non_exhaustive]`
gate on `DocumentAction` ensures any new I/O variant must add a
matching dispatcher carve-out.

**Dispatch status per gesture.** `DoubleClick`, `MiddleClick`,
`LeftDrag`, `WheelUp`, `WheelDown` are dispatched through
`dispatch_action` from their respective handlers. `LeftClick` and
`RightClick` are reserved tokens — the parser accepts them so user
configs don't fail validation, but no handler currently looks up
an Action for them. A single left-press is already consumed by the
selection state machine; wiring `LeftClick` would need a clear
post-selection dispatch point. `RightClick` has no non-color-picker
dispatch site at all.

**`LeftDrag`** is the continuous "press + movement past threshold
on empty canvas" gesture (default `PanCanvas`). The threshold
cross dispatches whatever Action the gesture resolves to — no
`PanCanvas` special-case in the handler — and the `PanCanvas` arm
sets `DragState::Panning` for the press duration, unless the state
it finds owes a commit (see the guard under
[`DragState`](#dragstate); a `pan_canvas` binding on a plain key
is why the guard is on the arm). The per-frame
pan delta stays inline in `event_cursor_moved.rs` because
per-cursor-move state is legitimately not a discrete-action
concern; the threshold frame's first delta is gated on the
dispatch having actually entered `Panning`, so an Action rebound
onto `LeftDrag` doesn't get a free camera nudge.

**Modifier-fallback for mouse gestures.** Mouse handlers resolve
through `ResolvedKeybinds::action_for_gesture`, which tries the
exact `(key, ctrl, shift, alt)` binding first and falls back to
the unmodified `(key, false, false, false)` binding if no exact
match exists. Modifiers on mouse gestures are typically decorations,
not distinct bindings — pre-branch `Ctrl+Wheel` zoomed exactly the
same as a bare `Wheel`, and the fallback preserves that. Users who
*do* want `Shift+DoubleClick` to mean something different just
bind it explicitly.

**Default-off `CreateOrphanNodeAndEdit`.** Empty-canvas double-click
ships unbound. Users opt back in via:

```json
{ "create_orphan_node_and_edit": ["DoubleClick"] }
```

**Custom-mutation parity.** `dispatch_custom_mutation_for_key`
mirrors the click-trigger path at `click.rs:35-64` byte-for-byte:
animation-aware (`start_animation` when `timing.duration_ms > 0`),
always invokes `apply_document_actions`. Closes the silent feature
gap where keyboard-triggered custom mutations skipped both.

---

## §6 The authoring surface

Authoring surface concepts are the parts a user actually touches:
modal editors, the console, keybinds, the color picker, clipboard,
and (briefly) the `maptool` CLI. Most are native-only today; the
parity story for each is honest.

### Inline node-text editor

Multi-line, grapheme-aware text editing on a
selected node; commit-on-click-outside, cancel on Esc.

Editing a node's text without leaving the
canvas. Double-click or Enter opens the editor; Backspace on a
selected node opens it pre-cleared; arrow keys move the cursor
in grapheme units. Live edits paint through a `DeltaGlyphArea`
mutation against the tree (not the model) so the user sees
in-flight characters; on commit, the model is updated and a
single `EditNodeText` undo entry is pushed.

`src/application/app/text_edit/mod.rs:29-80`.
Cross-platform — works on both native and WASM. Cursor math
runs on grapheme-cluster indices throughout (via
[`grapheme_chad`](#utilities--grapheme_chad-color-geometry)),
so emoji and combining marks behave as single units. Original
text and regions are snapshotted on open; Esc restores them.

### Inline single-line editor

Single-line text editing for the two one-line strings the model
carries: a line-mode edge's label, and a portal endpoint's
caption.

Setting or changing either without leaving the canvas. Same
lifecycle as the node editor (commit on click outside, cancel on
Esc) but restricted to one line. Portal captions are
per-endpoint: selection and editing target one endpoint at a
time, and the other endpoint's caption is unaffected.

`src/application/app/single_line_edit/`. One `SingleLineEditor`
holds `{target, buffer, cursor_grapheme_pos, original}`; a
`SingleLineEditTarget` variant owns everything that differs
between the two — where the current value lives, which preview
slot on `MindMapDocument` feeds the renderer
(`label_edit_preview` vs `portal_text_edit_preview`), which
canvas role is re-projected per keystroke, which setter commits,
and what "the release landed back on the thing I am editing"
means. Adding a third single-line editable is a variant plus one
arm per method; the lifecycle, the modal steal, the
click-outside commit and the dispatch arms do not grow.

The lifecycle core is renderer-free and returns an `EditRefresh`
naming the canvas work it owes, so the whole open / type /
commit / cancel sequence is driven directly in tests (§T8 keeps
live wgpu out of the harness).

One asymmetry survives on purpose: a portal caption stops being
editable once its edge is deleted or leaves portal mode, and a
mid-edit keystroke then closes the editor without committing.
The edge-label target has never had that guard and keeps typing
into a buffer whose edge is gone; its commit no-ops in
`set_edge_label`. `SingleLineEditTarget::still_editable` is where
the two answer differently, and the differential-oracle tests pin
both columns.

The `keybinds.json` vocabulary keeps its `label_edit_*` spelling
(`Action::LabelEdit*`, `InputContext::LabelEdit`) — those are
user-facing binding names, and both targets have always shared
them.

Native-only today. WASM users reach the same operations via the
`label` console verb, which has full cross-platform parity.

### Modal editor ladder

The steal / release shell both inline text editors sit in.

While a text editor is open it owns the keyboard: the key
resolves in that editor's input context, its commit / cancel pair
goes through the `dispatch_action` funnel, and everything else
reaches the editor's own handler as a literal `winit::Key`. On a
pointer release, a release inside the edited element keeps
editing and consumes the release; a release outside commits
through the funnel and lets the click route normally so the new
selection lands.

`src/application/app/modal_editor.rs`. `ModalEditor` is that
shell written once, over the node text editor and the single-line
editor; `event_keyboard.rs` has one steal block and
`event_mouse_click.rs` one click-outside-commit block rather than
three each. Steal order prefers the single-line editor; the
release ladder resolves the node text editor first. Both orders
are pinned by tests, because they are the kind of caller-level
contract a unit test on either editor cannot see.

### Glyph-wheel color picker

A modal HSV picker rendered as a 24-glyph hue ring
with sat/value crosshairs and theme-variable quick-pick chips.

Picking a color for the current selection
without leaving the canvas. Hover live-previews through the
`color_picker_preview` transient on `MindMapDocument`; the
connection, label, and portal passes read it during projection
and substitute the preview color for the targeted element. Click commits, click
outside cancels. Keyboard: h/H nudges hue, s/S sat, v/V value,
Tab cycles theme chips, Enter commits, Esc cancels.

`src/application/color_picker/mod.rs:1-77`
and `src/application/color_picker_overlay/`. Native-only today.
`compute_color_picker_layout()` is a pure function over
geometry + viewport, so layout can be unit-tested without GPU.
Two modes: contextual (modal, opened from edge context menu;
commits to the targeted edge and closes) and standalone
(persistent palette, opened via `color picker on`; commits to
the current selection and stays open).

Commit, cancel and the six HSV nudges are `Action`s that run in
`dispatch_action` like every other user-named effect
(CODE_CONVENTIONS §3), so a macro step can drive the picker.
`color_picker_flow::picker_op_for` is the single predicate the
funnel arm, the keyboard pre-filter and the click router share.
Only the picker's Copy / Paste / Cut stay modal-local — they
carry a hex payload no `Action` body can express.

### `BorderPreview`

A transient slot on `MindMapDocument` that stages border-config
edits without writing the model — the renderer substitutes the
preview style for the targeted node / section / canvas slot
while the slot is `Some(...)`, and the user terminates with
`commit` (writes through the matching committing setter) or
`cancel` (discards).

Authoring iteration on the four border surfaces (per-node,
per-section, two canvas defaults). Without preview, every kv
edit is a commit-then-undo cycle and the visual feedback comes
*after* the model write — the "creative toolkit" framing
depends on the user seeing changes before they land.

`src/application/document/nodes/border.rs` (the slot type +
setters) and `lib/baumhard/src/mindmap/tree_builder/overrides.rs`
(the borrowed view + injection in `border.rs` / `node_clip.rs`,
`section_frame.rs`). Same discipline as `ColorPickerPreview`:
never serialized, never push undo, never flip `dirty`. Cancel
or commit clears the slot; a fresh `set_border_preview` call
replaces the prior preview atomically. Selection drift causes
lazy defer-clear: the preview stops rendering when the live
selection no longer covers the target, and the actual slot
clear happens at the next `set_*` / `commit_*` / `cancel_*`
call. Implicit cancel: any of the four committing setters
clears the preview as their first line, so a non-preview edit
always wins. `Action::SetBorderPreview { target_kind:
BorderPreviewTargetKind, field, value }`,
`Action::CommitBorderPreview`, `Action::CancelBorderPreview`
expose the keybind / dispatch surface; `Esc` cancels through
`Action::ExitMode`'s body before mode-clear.

### `SectionFrameElement` and section-frame chrome

The cyan rectangles drawn around an active node's sections
while the user is in `InteractionMode::NodeEdit { node_id }`.
Each section gets one rectangle keyed on `(node_id,
section_idx, focused)`; the frame is heavier (or
canvas-default-overridden) on the section currently being
text-edited so the user sees which section their keystrokes
land in.

The chrome is a parallel canvas — it doesn't belong to the
node's own `GfxElement` tree, so a node move or text rebuild
doesn't re-emit the frames. The dedicated canvas role
`CanvasRole::SectionFrames` registers its own
`InPlaceMutator` slot; rebuild-or-skip dispatch keys on
`section_frame_content_signature(elements) -> u64` which
streams every field the projection reads directly into a
hasher (no intermediate Vec) so the signature comparison runs
allocation-free per `Plan §7.4`. This role needs only the one
signature because it has no in-place mutator body to protect —
see [canvas-role projection](#canvas-role-projection) for the
two-signature shape borders and portals use instead.

`lib/baumhard/src/mindmap/tree_builder/section_frame.rs` holds
all three halves — the `SectionFrameElement` shape, the
`build_section_frames` emission pass, and the tree builder +
content hasher. Three style cascades
feed the resolution: per-section `frame_border` →
`canvas.default_section_frame_border` (or
`default_focused_section_frame_border` for focused) →
hardcoded floor. Each cascade is editable through the
`section frame …` and `canvas section-frame [focused] …`
console verbs, plus the `BorderPreview` lifecycle.

### Console

A CLI-style command palette (Ctrl+;) for mutations,
styling, settings, and document operations.

Power-user operations that don't have a
keybind. The console covers the long tail: zoom-bound
authoring, font-size clamps, palette swaps, mutation listing
and application, FPS toggle. Tokenized shell-style
(whitespace-split, `"quoted"` preserves spaces, `key=value`
first-class). Tab-completion is contextual and prefix-matched;
scrollback shows command history with dimmed older lines.

`src/application/console/mod.rs:1-170`.
Native-only today. Verbs include `zoom`, `font`, `color`,
`label`, `edge`, `portal`, `anchor`, `body`, `cap`, `spacing`,
`fps`, `mutation` (with `list`, `help`, `apply`, `inspect`
subverbs), `open`, `new`, `quit`, `save`. Visuals borrow
`baumhard::mindmap::border::BorderGlyphSet::box_drawing_rounded`
for the frame; content is clipped via
`grapheme_chad::truncate_to_display_width` so wide CJK
characters never overflow.

### `Grammar` — the declarative console verb spec

**Every verb is one declaration, and one engine reads it.**
`console::spec` holds `Grammar` / `Subverb` / `Key` / `Slot` /
`Form` / `Vocabulary`, and `Command` carries a
`&'static Grammar` and nothing else: the usage forms `help`
prints, the `keys:` block under them, the search tags, the
completion popup, the kv parse loop and every hint all derive
from it. Adding a kv key is one table row — parse, complete,
help and hint follow.

A `Grammar` is a **level**, not a verb. `border` is one,
`border preview` is another, `canvas section-frame focused` a
third, each joined to its parent by a `&'static` reference on
`Subverb::child`. `subverb_sets` and `key_sets` are slices *of
slices*, so `canvas border` names border's fifteen keys and
border's seven per-field subverbs rather than transcribing
them; `section frame` names the same fifteen and adds its own
`section=`.

Four things the shape is load-bearing for:

- **`Vocabulary` answers three questions from one
  declaration** — the rows a popup offers, the `<…>` a usage
  form prints, and the word list an error can quote back.
  `Rows` covers both the document-derived vocabularies (a
  map's palettes, a node's sections, the host's font families)
  and the *suggestion* lists too open-ended to print: `zoom
  min=` offers eight zoom levels and accepts any positive
  float, so the popup gets the eight and the usage line gets
  `<zoom|unset>`.
- **A `Form` is (slots, keys), and a subverb may declare
  several.** `section move` takes `dx=`/`dy=` *or* `x=`/`y=`;
  `section resize` takes the `fill` literal *or* `w=`/`h=`.
  `help` prints one line per form, and the popup offers the
  union of the forms the line's positionals still admit —
  which is what puts `fill` beside `w=` and `h=` at
  `section resize <TAB>` and takes both away again at
  `section resize fill <TAB>`.

  The engine enforces the exclusion as far as the *slots*
  express it, and no further. `fill` sits in a slot only one
  form declares, so typing it rules the other form out and
  `section resize fill w=99` is refused by name and pointed at
  the shape that reads `w=`. `section move`'s two forms differ
  only in their *keys*, so an empty positional list admits
  both equally; deciding between them would mean letting
  whichever key was typed first pick the form. That exclusion
  stays a handwritten guard in `execute_move`, with a message
  naming both shapes — one of the two bespoke semantics #27's
  fix plan names as staying behind the table.
- **No verb sees a raw token index.** `spec::descent` is the
  only reader of token order. A handler asks
  `descent.subverb()`, `descent.parent_name(0)` and
  `descent.slot_value(args)`. Eight completion sites used to
  do their lookahead with `tokens.get(N)` while the arms they
  fed were keyed on the *positional* index, and the two
  disagree the moment a kv pair sits earlier on the line. The
  positional-vs-kv discriminator lives there too, declared on
  the subverb (`Subverb::gated`) rather than re-asked at each
  slot that emits the vocabulary.
- **A kv the matched form does not read is refused by name.**
  `kvs::read` asks per *form* — narrowed by the positionals
  already on the line, per the bullet above — rather than per
  level, and points the key at the form that does read it:
  the level's composed form for `border preset heavy
  color=#fff`, another shape of the same subverb for
  `section resize fill w=99`. Before the declaration both
  staged what they matched and discarded the rest without a
  word, the first identically on four surfaces.

What stays hand-written is value parsing and mutation: a `Key`
declares its name, its sentence and its vocabulary, and what
`padding=8` *means* is the verb's. Bespoke semantics —
`section move`'s mutual exclusion, the `border side`
non-custom-preset gate — are handwritten handlers *behind* the
table.

`console::spec`'s own tests hold the declaration against itself
over the whole registry, in both directions: a form naming a
key its level does not declare, and a key no form prints. The
second is what would have caught `font range=`, `color range=`
and `color section=` on the day each was written — each was
parseable, named in its verb's own rejection, and documented
nowhere.

They also hold the *engine* against the declaration, over the
same registry rather than a sample of it: for every form of
every subverb at every level, the line that reaches it is built
from the level's `label` and the form's own required slots,
handed to the real completion engine, and checked for the keys
that form names. Two more read the sources rather than the
tables, because a verb can decline to consult them: no verb
spells out a `usage:` line of its own (`label` did, two keys
behind its grammar), and no hint names a value in a `|`
alternation that the vocabulary beside it rejects
(`border color`'s two hints said `preset`, which the slot
refuses, and omitted `accent|edge|fg`, which it accepts).

The differential oracle (`console::tests::oracle`) is what made
the migration reviewable: `EXEC_CORPUS`, `COMPLETION_CORPUS`
and `EXEC_PREFIX_CORPUS` pin execute outcomes and popup
contents byte for byte, so a change that alters no pinned row
is behavior-preserving by construction and a row that does move
is a decision with a reason attached.

**A command reaches the document through two different calls,
and which one it picks is the rebuild signal.** `ConsoleEffects`
hands out `document()` (shared) and `document_mut()`
(exclusive); the second raises `document_mutated`, which
`console_input/exec.rs` reads back in
`console_line_needs_rebuild` to decide whether the line owes a
`scene_cache.clear()` + `rebuild_all`. The other input is the
command's `ConsoleSideEffect`: every variant counts except
`SetFpsDisplay`, whose overlay is screen-space and shares no
state with the scene tree. So `help`, `fps`, `mutation list`
and any verb that fails after only reading no longer drop the
connection cache and re-project the whole scene for output that
never leaves the scrollback.

The signal is the *borrow*, not the write. It over-reports — a
`border reset` that turns out to be a no-op still counts — and
that is the safe direction, since the cost is one rebuild
against a canvas silently disagreeing with the model. It is
also the version a new command cannot forget: there is no way
to write to the document except through the call that raises
it.

Console parity on WASM is the obvious next step;
the verb implementations are already cross-platform, only the
modal shell is native-gated.

### Keybinds and `Action`

A three-layer pipeline: abstract `Action` enum →
parsed `KeyBind` → resolved table; with cross-platform
configuration via XDG (native) and `localStorage` (WASM).

Every keystroke that does *anything* maps to
an `Action` first; the `Action` is then dispatched in the right
input context (Document, Console, ColorPicker, LabelEdit,
TextEdit). This indirection means users can rebind keys without
touching code, and the same `Action` works on both targets even
though the config-loading paths differ.

`src/application/keybinds/`. The three
layers:

- `Action` enum (`action/mod.rs`) — high-level intents:
  `Undo`, `CreateOrphanNode`, `EnterReparentMode`,
  `EnterConnectMode`, `DeleteSelection`, `EditSelection`,
  `OpenConsole`, `Copy`, `Paste`, `Cut`, `ExitMode`, etc.
- `KeyBind` parser (`bind.rs`) — string syntax like `"Ctrl+Z"`
  → modifier mask + key code.
- `ResolvedKeybinds` (`resolved.rs`) — fast `O(1)` lookup
  table built from `KeybindConfig` at startup.

`Action::context()` returns the input context the action
belongs to; the event loop uses it to filter eligible actions
based on which modal is open. Native config: hardcoded defaults
+ `$XDG_CONFIG_HOME/mandala/keybinds.json` + optional
`--keybinds <path>` CLI override. WASM: same defaults +
`localStorage["mandala_keybinds"]` — there is deliberately no
`?keybinds=` layer, because a query param is owned by whoever
composed the link rather than by the user (see
`keybinds/platform_web.rs`). Partial configs merge via serde
`default` attributes.

**The config surface is declared once.** A bindable Action used
to be written out three times — a `KeybindConfig` field, a
`Default` entry, a `resolve()` row — and a field missing from
`resolve()` compiled, leaving a binding that deserialized,
round-tripped and did nothing. `keybinds/surface.rs`'s
`keybind_surface!` collapses the three into one row per Action
in `keybinds/config.rs`, generates the struct itself, and emits
an `ActionKind → BindSurface` match that the loader's
recognized-key set is built by walking. Because that match is
exhaustive, an `Action` variant that appears in no section of
the table — neither bound nor listed under `unbindable` with a
reason — fails to compile. Unit variants take the string-list
shape (`"undo": ["Ctrl+Z"]`) and payload variants the `args`
shape; which one is not a choice, since each section only
accepts the variant shape it is for. Unrecognized top-level keys
in a user's file are warned about (`"keybinds: unrecognized key
…"`, naming the nearest recognized key when there is one) and
skipped rather than rejected — one stale *key* must not cost the
user the rest of their bindings — and keys starting with `_` are
treated as comments, which is how the shipped
`config/default_keybinds.json` carries its instructions. That
tolerance stops at the key: a *malformed value* under a
recognized key fails the whole serde pass, and
`user_config::layered::load_layered` answers a failed parse by
skipping the layer, so it costs the user every binding in the
file. The asymmetry is real rather than designed — the three
user configs share the seam — and issue #129 tracks the
per-field parsing that would close it.

**Parametric Actions.** A subset of variants carries payload
(`String` paths, `(field, value)` tuples, etc.) — these wrap
parameterized console verbs so a user can bind e.g. `Ctrl+B` →
`SetBorderField { field: "preset", value: "rounded" }` directly
in `keybinds.json` without authoring a macro. Bindings use a
sibling `ParametricBinding` shape:

```jsonc
{
  "set_border_field": [
    { "combo": "Ctrl+B", "args": ["preset", "rounded"] }
  ],
  "set_color": [
    { "combo": "Ctrl+1", "args": ["bg", "#fafafa"] },
    { "combo": "Ctrl+2", "args": ["text", "accent"] }
  ],
  "set_font": [
    { "combo": "F8", "args": ["size", "14"] }
  ],
  "set_zoom": [
    { "combo": "F12", "args": ["min", "0.5"] }
  ],
  // `clear_zoom` carries no payload, so it takes the plain
  // string-list shape every unit Action takes.
  "clear_zoom": ["Shift+F12"]
}
```

Color / font / zoom carry the axis as the first arg (`bg|text|border`,
`size|min|max`, `min|max` respectively) so a single binding-list
covers the whole field group. The typed `ColorAxis` / `FontSlot` /
`ZoomBound` enums on the Action variant make the dispatcher's
match exhaustive without a fan-out guard, and they reach the
payload through `surface::ArgValue`: a `String` field takes the
argument verbatim, a typed field goes through its strum `FromStr`,
and a payload type with no `ArgValue` impl does not compile.

Each variant names its arg shape in the table row that declares
it — the payload field names there are both the arity and the
list quoted back at the user when a binding's `args` array is
the wrong length. **Their order is the positional `args`
contract**, and the language does not enforce it on its own: the
generated constructor is a struct expression, which does not
care what order its fields are written in, so a row whose names
are transposed would compile and hand the user's arguments to
the wrong fields.

Two mechanisms hold that line, pinning different halves of it.
The build holds the *names*:
`mandala_derive::PayloadFieldNames` publishes each variant's
field names in declaration order into `action_payload_fields`,
`surface::keybind_field_order_check!` compares the row's names
against them under `const _: () = assert!(…)`, and a
transposition is an `error[E0080]` naming the row. Two
independent sources — the declaration and the table — so it is
not a mirror agreeing with itself, and a brand-new row is
covered with nothing written by hand. The tests hold the
*values*: the sentinel table in `keybinds/tests.rs` carries the
`Action` each row's args must produce, cannot omit a row, and
drives the args end to end through the JSON, the `ArgValue`
conversion and `resolve()` — which is where "`set_color`'s first
arg has to be a real `ColorAxis`" lives, and which the const
check, reading two lists of identifiers, cannot see.

Tuple rows (`SetEdgeBodyGlyph(glyph)`) sit outside the const
check by construction rather than by omission: their names are
local bindings the table invents, and the pattern that binds
them and the constructor that consumes them are the same macro
repetition, so there is no second order to disagree with.

Three skips are possible at resolve time and all three are
logged, never panicked: an unparseable combo, the wrong number
of args, and an arg that is not a valid value for its field.
The dispatch arms call `pub(crate)` mutation cores extracted
from each console verb, so the same setter path runs whether
the user types the verb or fires the bound key — including
`CycleBorderPreset` / `ToggleBorderVisible` (cores in
`console/commands/border/execute.rs`) and the font-size slots
(one selection dispatcher in `console/commands/font.rs`).
Section-targeted Action variants resolve their `(node_id,
section_idx)` through the shared cascade in
`console/commands/section/target.rs`, whose
`SectionTargetPolicy` names the two places the Action path
legitimately differs from the verb path (no document to count
sections, so no single-section auto-resolve; a genuine
multi-section selection is rejected rather than collapsed). Filesystem
variants (`OpenDocument`, `SaveDocumentAs`, `NewDocumentAt`) are
`NativeOnly` and denylisted from non-User macro tiers per the
privilege gate.

### User-tier config loading — `check_cap`, `read_capped`, `load_layered`

The three user-owned JSON files (`keybinds.json`,
`mutations.json`, `macros.json`) are all found the same way, so
the finding is written once in
`src/application/user_config/`.

- `MAX_USER_PAYLOAD_BYTES` (1 MiB) and `check_cap` are the single
  wording and enforcement of the size cap. A real config is a few
  KB; a multi-megabyte one is accidental or hostile and is
  rejected before serde ever sees it.
- `read_capped(path)` is the native filesystem read: stat, cap,
  read. Native-only, like its neighbor `xdg_mandala_path` — the
  filesystem tier of a user config exists only on desktop.
- `load_layered(label, layers, parse)` is the fallback walk over
  an ordered list of `ConfigLayer`s. Each layer is a name plus a
  lazy fetch; the first layer whose payload fits the cap *and*
  parses wins. An absent layer is skipped silently; a broken one
  is logged and the walk continues; exhausting the list returns
  `None` and the caller substitutes its defaults. Nothing here is
  platform-specific, so the precedence logic is unit-tested on
  native even for the browser's layers.
- Each target names its own layers exactly once, in the
  composition wrapper that sits on the driver:
  `desktop::load_desktop_layered(label, filename, explicit, parse)`
  for the explicit CLI path before the XDG path, and
  `web_storage::load_web_layered(label, param, key, parse)` for
  `?<param>=<json>` before `localStorage[key]`. All six platform
  loaders (three configs × two targets) are now a filename and a
  parser.
- The one deliberate asymmetry lives in the desktop wrapper: only
  the XDG layer is filtered on `exists()`. An absent user config
  is the normal case and stays silent, whereas an explicit
  `--keybinds <path>` that does not resolve is a user error worth
  a warning. Changing that is a change to one function.

Adding a fourth user-tier config file is a matter of naming its
filename, query param, and storage key, then handing each
wrapper a parser — no new read, cap, layer, or fallback code.

### `SourceTier`

The `App < User < Map < Inline` ladder in
`src/application/source_tier.rs`, shared by the custom-mutation
registry and the macro registry — both are id-keyed and both take
definitions from the same four places. The ladder means two
things at once: **precedence** (later tiers override earlier ones
on an id collision, which the derived `Ord` encodes) and **trust**
(`App` ships with the binary and `User` is the user's own file,
while `Map` and `Inline` arrive inside a possibly-shared
`.mindmap.json`). The macro dispatcher's privilege gates —
`allows_console_line`, `allows_action`, both `impl SourceTier`
blocks in `src/application/macros/mod.rs` — key off exactly that
trust split. Tier assignment is loader-pinned: nothing in an
on-disk file can raise its own tier.

### Clipboard

Cross-platform copy / cut / paste, with native
backed by `arboard` and WASM stubbed pending async-clipboard
integration.

Selection-routed clipboard: each
[`SelectionState`](#selectionstate) variant has its own channel.
Copying a node copies its style and text; copying a section
copies a structured payload (text + per-run formatting + offset
/ size / channel / bindings); copying an edge copies the body
color; copying an edge label copies the label color; copying
a portal label copies the icon color; copying a portal text
copies the text color. The font channel mirrors this routing
for `font size= min= max=` writes.

`src/application/clipboard.rs`. The OS
clipboard layer (native `arboard`, WASM stub) carries plain
text. A thread-local in-process `SECTION_BUFFER` slot carries
the structured `SectionPayload` for within-app section→section
round-trip; on paste, the payload is consulted only when its
`text` snapshot matches the OS clipboard's current text exactly
(consistency check; falls through to plain text when the user
copied from another app between Mandala copy and paste).
Failures (permission denied, unavailable) log via `log::warn!`
and return `None` — interactive paths must not panic. WASM
stubs warn-and-noop pending the browser's async clipboard API.

### `maptool` CLI

A separate binary in `crates/maptool/` for
scripted operations on `.mindmap.json` files: `show`, `grep`,
`apply`, `export`, `convert --legacy`, `convert --portals`,
`convert --sections`, `verify`.

Authoring and maintenance from outside the
app. `verify` is the structural-invariant checker
([`format/validation.md`](./format/validation.md)). `convert`
migrates legacy formats — `--legacy` runs the portal and section
folds inside itself, so a miMind import is one hop
([`format/migration.md`](./format/migration.md)); every verb writes
through an atomic staging file + rename, so input and output may be
the same path. `apply` pipes node text through an
external command for batch edits. `export` renders to Markdown.
`grep` and `show` are read-only inspectors.

`crates/maptool/`. Not the focus of this
document — see the crate directly for the verb-level reference.
The format docs under [`format/`](./format/) are the
authoritative reference for what `verify` enforces.

---

