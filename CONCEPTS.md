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

### How to read an entry

Each concept has up to four layers, in this order:

- **Summary.** One sentence. Plain words where possible.
- **What it's for.** Two to five sentences. The problem it solves
  and where a user or developer encounters it.
- **Under the hood.** The technical side: file locations, costs,
  invariants, and the small surprises. Skip this layer unless you
  need it.
- **Vision.** Where this is known to be going. Honest about what
  is not built yet.
- **Caveat.** Non-obvious gotchas that have bitten people.

Concept names appear in `backticks`. File references use
`path/to/file.rs:line` so you can jump to them. Links into
`format/*.md` point at the authoritative JSON-surface reference for
each concept; this document does not repeat their field-by-field
detail.

---

## Table of contents

- [§1 Project foundations](#1-project-foundations)
- [§2 The Baumhard foundation](#2-the-baumhard-foundation)
- [§3 The mindmap domain](#3-the-mindmap-domain)
- [§4 The mutation framework](#4-the-mutation-framework)
- [§5 The application runtime](#5-the-application-runtime)
- [§6 The authoring surface](#6-the-authoring-surface)
- [§7 Platform & parity](#7-platform--parity)
- [§8 Named trajectory — vision](#8-named-trajectory--vision)
- [§9 Glossary index](#9-glossary-index)

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

Any change to the data model is expressed as a **mutator** applied to
a **tree** ([§2: `Tree`](#treet-m), [§2: `MutatorTree`](#mutatortreem),
[§2: `Applicable`](#applicablet)). Clone-edit-reinsert is never the
shape of a change. This is the single most pervasive discipline in
the codebase: it is what makes incremental updates cheap, what lets
user actions compose into undo/redo cleanly, and what gives custom
mutations ([§4](#4-the-mutation-framework)) their reach.

### Everything is glyphs

Text, borders, connection lines, portal markers, console chrome,
selection highlights — every visual element is a positioned font
glyph. There are no rectangle-shader UIs, no bitmap sprites, no
icon atlases. Introducing a new kind of visual is therefore a
question of "what glyph goes where", not "add a new pipeline". A
new rendering pipeline is a project-scale decision.

### Single-threaded event loop

`Application` owns the `Renderer` directly. There are no channels,
no worker threads, no `tokio`, no `std::thread::spawn` in any
interactive path. The one sanctioned exception is the native
[`FreezeWatchdog`](#freezewatchdog) thread, which only *reads* a
`AtomicU64` ping from the main loop. The single-threaded invariant
is what makes the model simple, what makes lock scopes trivial, and
what makes the whole system easier to reason about than a typical
engine.

### Model / view separation

The [`MindMapDocument`](#mindmapdocument) owns the data: `MindMap`,
selection, undo stack, animations, mutation registries. The
[`Renderer`](#renderer) owns GPU resources. The renderer reads
intermediate representations (`Tree<GfxElement, GfxMutator>`,
`RenderScene`) from the document on each frame; it never reaches
into the document. The document never holds GPU handles.

### Cross-platform as first-class

Native desktop, browser on desktop, and browser on mobile are three
equally supported deployments. The lowest-spec target (mid-range
phone in a browser) sets the performance budget. New interactive
features ship cross-platform from the start; native-only additions
are a declared parity gap, not a style choice. `cfg`-guards at the
module boundary are the only abstraction used — traits would have
been the wrong shape. See
[`CODE_CONVENTIONS.md §4`](./CODE_CONVENTIONS.md) and the
"Dual-target status" section of [`CLAUDE.md`](./CLAUDE.md).

### Canonical or exemplary

The bar for every merged change is *canonical* or *exemplary*.
Nothing less. This is a public-domain project with no commercial
pressure; the measuring stick is whether the work is good enough for
God ([`CODE_CONVENTIONS.md §0`](./CODE_CONVENTIONS.md)). Concretely:
every commit is a state we would ship, tests green, no half-features
behind flags, no TODO comments, no dead code. "Not caused by my
changes" is not an excuse — if you notice a gap, you own the close.

### Preserved seams

A **seam** is a point in the surface where a future extension can
attach without rewriting what is around it: a `pub` boundary on a
primitive, a composable mutator variant, a registry that accepts
handlers, a field that exists today for a narrow purpose but whose
shape admits more. Seams are named here frequently. "Extra ceiling
height" is deliberate: it serves the *named trajectory* (plugins, a
Baumhard script API, richer animations, complex exports) even when
today's use is narrow. See [`CODE_CONVENTIONS.md §7`](./CODE_CONVENTIONS.md).

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

**Summary.** An arena-backed forest of typed nodes with cached
spatial indices, representing one layer of visual content.

**What it's for.** A `Tree` is how Baumhard stores anything
hierarchical that needs to render or be hit-tested: one tree for the
mindmap nodes, one for connection glyphs, one for borders, one for
the console overlay, and so on. Every node in a tree is reached
through an opaque `NodeId` — not a pointer, not an index — so the
tree can be rearranged, cached, or serialised without invalidating
references. Nodes are `Clone`, mutation is in-place (never
rebuild-the-arena), and both AABB caches and an optional region
index ride along so hit-testing stays cheap.

**Under the hood.** Defined in
`lib/baumhard/src/gfx_structs/tree.rs`. Wraps `indextree::Arena<T>`;
adds `root: NodeId`, `layer: usize`, an AABB cache (`Cell<Option<...>>`
because the values are `Copy`), a subtree-AABB dirty flag, an
optional `RegionParams` + `RegionIndexer` for spatial queries, a
`position: Vec2` offset, and a `pending_mutations` vector for
deferred application. The blessed iteration primitives are
`NodeId::children(&arena)` and `descendants(&arena)`; collecting
into a `Vec<NodeId>` is a code smell. Every `MutatorTree::apply_to`
call invalidates the AABB cache once, not per-field.

**Vision.** `position` and `pending_mutations` are seams: the first
admits multi-viewport rendering without rework, the second lets
event subscribers queue reactive mutations without fighting the
walker. Both are used narrowly today and preserved at full width.

### `MutatorTree<M>`

**Summary.** The mutation-side mirror of a `Tree`: same shape,
carrying deltas instead of values.

**What it's for.** If `Tree` is the *noun*, `MutatorTree` is the
*verb*. A `MutatorTree<GfxMutator>` describes a change to apply to a
`Tree<GfxElement, GfxMutator>`: "mutate the third child's text,
shrink the font on every descendant of channel 2, repeat until the
predicate fails." The tree walker pairs the two up by channel (or
sibling position, depending on the instruction), applies matching
deltas in place, and leaves the rest alone. This is the seam custom
mutations ([§4](#4-the-mutation-framework)) ride on.

**Under the hood.** Also in
`lib/baumhard/src/gfx_structs/tree.rs`. Minimal — an `Arena<T>` and
a `root: NodeId`. No spatial data: mutators are pure deltas, they do
not render. The trait bound `TreeNode` requires a `void()` sentinel
for padding the mutator's shape to match the target's when channels
do not line up. `MutatorTree<GfxMutator>::apply_to(&mut target)` is
the whole entry point; it calls `walk_tree_from` under the hood.

### `Applicable<T>`

**Summary.** A one-method dispatch trait: "apply this delta to that
value".

**What it's for.** Almost every mutation primitive in Baumhard
implements `Applicable` against its target type. `MutatorTree<M>
: Applicable<Tree<T, M>>` is the big one, but there are also
`DeltaGlyphArea: Applicable<GlyphArea>`,
`DeltaGlyphModel: Applicable<GlyphModel>`,
`GlyphAreaCommand: Applicable<GlyphArea>`, and so on. The shape is
always `fn apply_to(&self, target: &mut T)`. This keeps the
vocabulary uniform: to learn how a new delta works, look at its
`apply_to` and nothing else.

**Under the hood.** Defined in `lib/baumhard/src/core/primitives.rs`.
The trait is deliberately minimal; no associated types, no `Result`.
Interactive paths cannot panic
([`CODE_CONVENTIONS.md §9`](./CODE_CONVENTIONS.md)), so type
mismatches (e.g. applying a `ModelDelta` to a `GlyphArea` target)
are silently ignored by design — the dispatch site is responsible
for well-typed pairing. This tradeoff is ugly but correct for a
real-time editor: the cost of a dropped mutation is a visual
glitch on one frame; the cost of a panic is a lost document.

### `ApplyOperation`

**Summary.** The operation selector a delta carries — `Add`,
`Assign`, `Subtract`, `Multiply`, `Delete`, or `Noop`.

**What it's for.** A single `DeltaGlyphArea` does not hardcode
"text replace" vs. "text append"; it carries an `ApplyOperation`
that tells the generic `apply` helper which trait assignment to
use. That is how one `Text(String)` delta variant covers both
"concatenate this suffix" (`Add`) and "replace the whole text"
(`Assign`) without duplicating the variant.

**Under the hood.** `lib/baumhard/src/core/primitives.rs`. The
generic apply requires the target type implement `AddAssign`,
`SubAssign`, `MulAssign`, and `Default` — which is why every
mutable field type in the delta world carries all four.

### `GfxElement`

**Summary.** The tagged union every `Tree<GfxElement, _>` node is
— either a `GlyphArea`, a `GlyphModel`, or a `Void`.

**What it's for.** This is *the* tree-node type in the codebase.
All visual things — every text region, every composed-glyph
shape, every structural padding node — is one variant of this
enum. Shared metadata rides on every variant: a `channel` for
mutation routing, a `unique_id` assigned by the host app (Mandala
uses it for the mindmap node id), a `flags` set, an
`event_subscribers` list, and a cached `subtree_aabb`.

**Under the hood.** `lib/baumhard/src/gfx_structs/element.rs`.
`GlyphArea` and `GlyphModel` each box their payload (one heap
allocation per element of that kind); `Void` has no payload.
There is a companion `GfxElementType` enum for cheap variant
checks without destructuring, and a `GfxElementField` enum used
by predicates and field-level mutations to name "which part of
which variant".

### `GlyphArea`

**Summary.** A text region — the only element that actually draws
glyphs to the screen.

**What it's for.** When something visible has characters in it, a
`GlyphArea` represents it: mindmap node text, connection glyphs,
portal icons, console lines, FPS overlay digits. The struct
carries everything the renderer needs to shape and draw that text
— position, render bounds, font scale and line-height, per-span
color/font overrides ([`ColorFontRegions`](#colorfontregions)), a
background fill, an optional outline halo, a hit shape, and a
zoom-visibility window.

**Under the hood.** `lib/baumhard/src/gfx_structs/area.rs`. Uses
`OrderedFloat<f32>` and `OrderedVec2` for its numeric fields so
the struct is `Eq + Hash` despite holding floats — important for
caching and identity-based diffing. The `hitbox` field is the one
exception to the hash/eq contract: it is derived by the scene
builder from the rest of the fields, not part of identity. One
`GlyphArea` maps to one cosmic-text `TextArea` in the renderer.

**Caveat.** The `text: String` is edited with grapheme-aware
helpers ([`grapheme_chad`](#utilities--grapheme_chad-color-geometry)); byte
offsets from user-facing counts will land mid-cluster on the
first emoji.

### `GlyphModel`, `GlyphMatrix`, `GlyphLine`, `GlyphComponent`

**Summary.** A four-level composition hierarchy for glyph shapes
built out of small typed cells.

**What it's for.** Sometimes a visual element is more structured
than a plain string — a grid, a menu, a composed diagram built of
box-drawing pieces. `GlyphModel` is the answer: it is a child of a
`GlyphArea` that contributes a matrix of lines of components, each
component carrying its own text plus optional font and colour
overrides. The model paints its contents *into* the owning
`GlyphArea`'s buffer at shape time, so the whole thing shapes and
renders as one cosmic-text pass while remaining structurally
addressable for mutation.

**Under the hood.** The hierarchy is: `GlyphModel` owns a
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

**Summary.** A no-op tree node: no payload, no render cost, just
structure.

**What it's for.** Sometimes a mutator tree needs a child at index
*k* that does nothing, so subsequent children align against the
right target children. Sometimes a target tree needs a parent that
has no content of its own but holds other elements. `Void` is the
answer in both cases. It is never required — but used tastefully,
it keeps tree shapes regular and channel alignment clean.

**Under the hood.** `lib/baumhard/src/gfx_structs/element.rs` for
the target side; same enum on the mutator side in `mutator.rs`.
No heap allocation, just metadata (channel, id, flags).

### `ColorFontRegions`

**Summary.** A set of character-range spans, each with optional
colour and font overrides, layered over a `GlyphArea`'s text.

**What it's for.** A single node's text can have multiple styles —
a bold first word, a red annotation, a smaller footnote. Rather
than fragmenting text into per-style nodes, Baumhard carries
**span tables**: `[start, end)` ranges that say "between these
two positions, use this colour and/or this font". Any part of the
text not covered by a span inherits the area-level defaults.
The same primitive drives rich-text on mindmap nodes
([`text runs`](#text-runs)), highlight on selected regions, and
transient live-edit previews.

**Under the hood.** `lib/baumhard/src/core/primitives.rs`. Backed
by `BTreeSet<ColorFontRegion>` keyed on the `Range`, so lookups
by range are `O(log n)` but two regions with the same range and
different payloads collide (last write wins) — this is
deliberate, not a bug. The `Range` indices are **Unicode
code-point offsets** when the caller comes from mindmap text
runs (matching [`text runs`](#text-runs)); the primitive itself
just holds `usize` pairs and does not enforce a unit, so
consumers that reach in from elsewhere must agree on the same
convention. Five mutation primitives keep the set
consistent under text edit: `insert_regions_at`,
`shrink_regions_after`, `split_and_separate`,
`shift_regions_after`, `set_or_insert`. A spatial index
([`RegionIndexer`](#regionparams-regionindexer-regionerror)) can
be layered on top for hit-testing.

**Caveat.** Never mutate `ColorFontRegions` outside the mutator
pipeline — direct writes skip the index update and selection
drifts silently. See [`lib/baumhard/CONVENTIONS.md §B6`](./lib/baumhard/CONVENTIONS.md).

### `Range`

**Summary.** A half-open `[start, end)` span of `usize` indices.

**What it's for.** The canonical primitive for "some part of the
text" everywhere text appears. `ColorFontRegion` keys on it;
`GlyphAreaCommand::ChangeRegionRange` manipulates one; text-run
schema validation runs over them. Small but load-bearing: a
single shared `Range` type means span operations compose across
modules without glue.

**Under the hood.** `lib/baumhard/src/core/primitives.rs`. Totally
ordered for `BTreeSet` use; ships with `magnitude`, `push_left`,
`push_right`, `overlaps`, `to_rust_range`.

### `Channel` and `BranchChannel`

**Summary.** An integer routing tag on every node. The tree walker
matches mutator nodes to target nodes by equal channel within a
sibling group.

**What it's for.** Without channels, every mutation applied to a
parent would broadcast to *every* child; with channels, the author
can say "this mutation only hits siblings tagged channel 1". A
parent and its child can share a channel or differ; the matching
is within-sibling only. Siblings on the same channel form a
*broadcast group*: one mutator affects all of them. This is the
primitive that makes a single mutation selective without naming
child indices.

**Under the hood.** The `BranchChannel` trait
(`lib/baumhard/src/gfx_structs/tree.rs`) is a one-method trait
`fn channel(&self) -> usize`. Both `GfxElement` and `GfxMutator`
implement it. The walker calls it to align children. In the
mindmap domain, `MindNode.channel` is where this surfaces to
end users; see [§3: Channels](#channels-mindmap-level).

**Caveat.** Children arrive at the walker in **Dewey-id order**
(`id_sort_key`), not in channel order — the tree builder sorts
by id, not by channel. Channel matching happens within whatever
sibling order the map defines. Authoring custom mutations that
target specific channels therefore means arranging children so
that channel order and id order agree, or reaching for the
[`MapChildren`](#instruction) instruction to pair strictly by
sibling position instead.

### `Flag` / `Flaggable` / `AnchorBox`

**Summary.** A small enum of state markers any node can carry, and
the trait that queries them.

**What it's for.** Some per-node state is not *data* in the
rendering sense but *status* — "this node is focused", "this
node is in edit mode", "this node is anchored to a specific
screen corner". Flags provide a uniform place to store those,
queryable by [predicates](#predicate-and-comparator) without
extending the element's data fields.

**Under the hood.** `lib/baumhard/src/core/primitives.rs`. Current
variants: `Focused`, `Mutable`, `Anchored(AnchorBox)`,
`MutationEvents`. `AnchorBox` holds up to four `Anchor` entries
for layout-solver pinning. `MutationEvents` is reserved — it
marks a node that should fire events on mutation (a seam for
future reactive handlers).

### `Event`, `GlyphTreeEvent`, `GlyphTreeEventInstance`, `EventSubscriber`

**Summary.** A *non-state-mutating* kind of mutator: instead of
changing element data, it invokes callbacks subscribed to the
element.

**What it's for.** Event-driven behaviour (button-like nodes,
hover-response, keyboard dispatch to a focused node) does not
belong in the mutation-first data pipeline — a keystroke is not
a delta to a field. Events reuse the mutator infrastructure for
dispatch but invoke subscriber callbacks instead of editing
data. A subscriber *can* enqueue further mutations as a
reaction, which is how reactive chains are built.

**Under the hood.** `lib/baumhard/src/gfx_structs/mutator.rs`.
`GlyphTreeEvent` is the enum of event kinds (`KeyboardEvent`,
`MouseEvent`, `AppEvent`, `CloseEvent`, `KillEvent`);
`GlyphTreeEventInstance` wraps it with a timestamp;
`EventSubscriber` is
`Arc<Mutex<dyn FnMut(&mut GfxElement, GlyphTreeEventInstance)
 + Send + Sync>>`. The `Arc<Mutex<…>>` shape exists so that
cloning an element (as the arena does) keeps a single callback
reachable from every clone rather than duplicating state.

**Vision.** Today the mindmap app does not use subscribers
heavily — most interaction goes through the application's own
input handlers. The seam is preserved for the Baumhard script
API and plugin trajectory, where user-authored code will want
to subscribe to events without reaching into the app crate.

### `Predicate` and `Comparator`

**Summary.** A small expression language for "does this element
match?" tests, used by loop and dispatch instructions.

**What it's for.** Some mutations only apply to certain nodes —
"every child whose font size is under 12pt", "every descendant
marked `Focused`". A `Predicate` names the fields to test and
the `Comparator` (equals, not-equals, greater-than, etc.) to use
against each; the walker evaluates it per candidate node and
decides whether to recurse.

**Under the hood.** `lib/baumhard/src/gfx_structs/predicate.rs`.
Pure data (serialisable); typical predicates carry one or two
fields, so evaluation is effectively `O(1)`. Float comparisons
use `almost_equal` with a `1e-5` epsilon
([`util/geometry.rs`](#utilities--grapheme_chad-color-geometry)). The
`Comparator` uses a *negation flag* pattern: `Equals(false)` is
`==`, `Equals(true)` is `!=`, halving the variant count.

### `Instruction`

**Summary.** The four control-flow primitives a `GfxMutator` can
carry: `RepeatWhile`, `SpatialDescend`, `MapChildren`, and
`RotateWhile` (reserved).

**What it's for.** Most mutations are direct: apply this delta to
this node. Some need to loop ("apply this to every descendant
matching predicate X"), some need spatial routing ("apply this
to whichever node contains this point"), some need
position-indexed pairing ("apply these N mutators to these N
siblings, zip-style"). `Instruction` is the vocabulary. This is
how one custom mutation can sweep a whole subtree without
hand-listing every target.

**Under the hood.** `lib/baumhard/src/gfx_structs/mutator.rs`.
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

**Summary.** The mutator-side node type, mirroring `GfxElement`:
`Single`, `Macro`, `Void`, or `Instruction` variants.

**What it's for.** Every node of a `MutatorTree` is a
`GfxMutator`. The four variants cover "one field change here",
"a batch of changes on this target", "structural padding", and
"control flow with nested children". Together with
[`Instruction`](#instruction) and
[`Predicate`](#predicate-and-comparator) they form a small but
complete mutation language.

**Under the hood.** `lib/baumhard/src/gfx_structs/mutator.rs`.
Implements `BranchChannel`. The `Mutation` payload can be an
`AreaDelta`, `AreaCommand`, `ModelDelta`, `ModelCommand`,
`Event`, or `None`. A `Macro` carries a `Vec<Mutation>` applied
in order to the same target, plus optional `children` for
descendant instruction nodes.

### `Mutation` enum

**Summary.** The payload union: which kind of delta or command
this mutator carries.

**What it's for.** A mutation is not one uniform thing — a
`GlyphArea` and a `GlyphModel` accept different kinds of change.
The `Mutation` enum is the sum type covering all of them: two
flavours each for area and model (field-level `Delta` vs.
imperative `Command`), plus `Event` (subscriber dispatch) and
`None` (structural placeholder).

**Under the hood.** `lib/baumhard/src/gfx_structs/mutator.rs`.
Each variant boxes its payload to keep the enum compact. Type
mismatches (e.g. `ModelDelta` applied to a `GlyphArea`) are
silently ignored per the [`Applicable`](#applicablet) no-panic
rule.

### `GlyphAreaField` and `DeltaGlyphArea`

**Summary.** The per-field delta surface for `GlyphArea`: text,
scale, position, bounds, regions, outline, shape, zoom
visibility.

**What it's for.** This is the granular surface any field-level
mutation reaches into. A font-size change is a
`GlyphAreaField::Scale(…)` inside a `DeltaGlyphArea` with an
`ApplyOperation` — that one pattern scales across every field
without bespoke plumbing per field.

**Under the hood.** `lib/baumhard/src/gfx_structs/area_fields.rs`
for the field enum and `OutlineStyle`; `area_mutators.rs` for
`DeltaGlyphArea`. The wrapper carries one `ApplyOperation`
shared across all fields in the batch, so "move this node 10
units right" and "set this node's text" use the same delta type
with different field lists and a different operation.

### `GlyphModelField`, `DeltaGlyphModel`, `GlyphModelCommand`

**Summary.** The `GlyphModel` mutation surface — the parallel of
the area-side delta and command trio, applied to composed-glyph
structures rather than plain text.

**What it's for.** Everything the area side offers
([`GlyphAreaField`](#glyphareafield-and-deltaglypharea),
[`GlyphAreaCommand`](#glyphareacommand)), the model side needs
too — position nudges, matrix inserts and replacements, colour
and font edits on individual components. Same operation vocab
(`ApplyOperation`), same `Applicable` dispatch, same walker
path; different target type.

**Under the hood.**
`lib/baumhard/src/gfx_structs/model/mutator.rs`. `GlyphModelField`
variants cover the structural bits (matrix inserts, component
edits, model position). `DeltaGlyphModel` wraps them with an
`ApplyOperation`. `GlyphModelCommand` is the named-operation
counterpart for things that don't fit arithmetic — row pops,
matrix-coordinate moves, rotations. All three ride in the
`Mutation::ModelDelta` / `Mutation::ModelCommand` variants of
[`GfxMutator`](#gfxmutator).

### `GlyphAreaCommand`

**Summary.** The *named-operation* mutation surface, for actions
that are not arithmetic deltas.

**What it's for.** Some operations have fixed semantics that
don't map to "add/subtract/assign": *pop the last three
graphemes*, *change the range of this region*, *delete a
specific region*. Commands are the vocabulary for those.
Imperatively named, grapheme-aware, covers ~16 operations.

**Under the hood.** `lib/baumhard/src/gfx_structs/area_mutators.rs`.
All grapheme-touching commands use `grapheme_chad` helpers, so
emoji / ZWJ / combining-mark sequences survive intact.

### `OutlineStyle`

**Summary.** A coloured halo behind text, rendered as eight stamp
copies (four cardinals + four diagonals) around the main glyph.

**What it's for.** When glyphs sit on a busy background, legibility
drops. `OutlineStyle` draws an outline halo so the glyphs read.
It is a field on `GlyphArea`, optional; default is no outline.

**Under the hood.** `lib/baumhard/src/gfx_structs/area_fields.rs`.
Two fields: `color: [u8; 4]` and `px: f32`. Cost is **9×** the
cosmic-text shapings of the area (one main + eight stamps). Hot
path, so enable only when background legibility demands it.

### `NodeShape`

**Summary.** A pluggable hit-test shape — `Rectangle` or
`Ellipse` — shared between the renderer SDF and the BVH
descent.

**What it's for.** A node's visual silhouette and its clickable
silhouette must agree. `NodeShape` names the two today
(rectangle, ellipse) and gives both pipelines one source of
truth for "is this point inside?". Adding a new shape is
three small changes: one enum variant, one WGSL shader `case`,
one `contains_local` arm.

**Under the hood.** `lib/baumhard/src/gfx_structs/shape.rs`.
`contains_local` does point-in-AABB or point-in-ellipse
(normalised coordinates, `nx² + ny² ≤ 1`); degenerate bounds
always return `false`. `intersects_local_aabb` supports
rect-select with conservative approximation for ellipses.

**Vision.** Shape-aware borders (glyph-drawn frames that follow
the ellipse outline, not just the AABB) wait on the
[`GlyphBorderConfig`](#border-geometry) side; the primitive
surface here is ready.

### `ZoomVisibility`

**Summary.** An optional inclusive `[min, max]` camera-zoom
window that gates whether an element renders at the current
zoom.

**What it's for.** Visual detail that makes sense at one zoom
rarely makes sense at another. A legend label is precious when
zoomed in on its region and noise when the whole map is on
screen; an overview landmark is a guide when zoomed out and
redundant up close. `ZoomVisibility` lets authors say "this
appears between 1.5× and 3× zoom" and have the renderer silently
honour it — no script, no custom mutation, just two fields.

**Under the hood.** `lib/baumhard/src/gfx_structs/zoom_visibility.rs`.
Two `Option<f32>` fields, a `contains(zoom) -> bool` predicate;
cost is two branchless float comparisons, benchmarked as
sub-nanosecond. No cosmic-text reshaping or buffer-cache
invalidation fires on zoom steps. At the mindmap layer the
surface is two flat fields (`min_zoom_to_render`,
`max_zoom_to_render`) on `MindNode`, `MindEdge`,
`EdgeLabelConfig`, and `PortalEndpointState`; see
[`format/zoom-bounds.md`](./format/zoom-bounds.md) and
[§3: Zoom bounds](#zoom-bounds).

**Caveat.** `NaN` zoom is treated as "not visible" deliberately —
a `NaN` camera is a bug upstream, and culling the frame surfaces
it faster than carrying the `NaN` through the glyph pipeline.
Inverted windows (`min > max`) render as "always hidden" at
runtime; `maptool verify` flags these as authoring errors.

**Vision.** The seam waiting here is **zoom-triggered LOD
mutations**: a `CustomMutation` bound to a zoom threshold could
swap a node's content entirely at the transition, so a cluster
summary becomes a detail view as you zoom in.
`GlyphAreaField::ZoomVisibility` already carries the mutator
target; what remains is the dispatcher that fires mutations on
zoom crossings.

### `Camera2D` and `CameraMutation`

**Summary.** A 2D canvas camera with pan/zoom and an intent-level
mutation vocabulary.

**What it's for.** The renderer projects canvas coordinates to
screen pixels through a `Camera2D`; pan and zoom are represented
as `CameraMutation` variants so that one handler can accept
input, animation, and scripted values uniformly. When a gesture
says "pan by 10 pixels" and an animation says "fit-to-bounds with
5% margin", both go through the same apply site.

**Under the hood.** `lib/baumhard/src/gfx_structs/camera.rs`.
Position in canvas space (the point at the viewport centre),
`zoom: f32` clamped between `MIN_ZOOM = 0.05` and `MAX_ZOOM =
5.0`. `CameraMutation` variants: `Pan { screen_delta }`,
`ZoomAt { screen_focus, factor }`, `ZoomCenter { factor }`,
`SetPosition { canvas_pos }`, `SetZoom { factor }`,
`FitToBounds { min, max, padding_fraction }`. Projection
helpers `canvas_to_screen` / `screen_to_canvas` are the only
place coordinate-space conversion lives.

### `Scene`

**Summary.** A multi-layer compositor: owns many `Tree`s at
different draw-order layers and screen-space offsets.

**What it's for.** The mindmap canvas is one tree; connection
glyphs are another; the console overlay is another; the color
picker overlay is yet another. `Scene` collects them all,
orders them by layer, and provides a single `component_at(point)`
hit-test entry that walks top-to-bottom and returns the first
tree that owns the point. This is the structural seam where the
`AppScene` at the application layer
([§5: scene host](#appscene-and-scene-host)) attaches.

**Under the hood.** `lib/baumhard/src/gfx_structs/scene.rs`. Uses
`Slab<SceneEntry>` for stable ids across insert/remove; each
entry carries `layer: i32`, `offset: Vec2`, and `visible: bool`.
Hit-test is `O(trees)` at the scene level and `O(tree size)`
inside the matched tree.

### `TreeWalker`

**Summary.** The recursive dispatch engine that walks a
`MutatorTree` against a `Tree` and applies matched mutations.

**What it's for.** Every mutation that ever lands on an element
goes through the walker — `MutatorTree::apply_to` just calls
`walk_tree_from`. The walker aligns children by channel (or by
position, depending on instruction), recurses, and dispatches
deltas to `Applicable::apply_to` at the leaves. Cost is `O(sum
of matching pairs)` — pruned branches are free.

**Under the hood.** `lib/baumhard/src/gfx_structs/tree_walker.rs`.
Key functions: `walk_tree_from` (the entry), `align_child_walks`
(the channel-based pairing), `process_instruction_node` (the
loop/spatial/map dispatch), `DEFAULT_TERMINATOR` (the closure
that resumes normal channel alignment after a `RepeatWhile`
exits). Branchless enough that matching-pair cost dominates.

### Mutator builder DSL — `MutatorNode`, `SectionContext`, `Repeat`, runtime holes

**Summary.** A serde-friendly AST (`MutatorNode`) that compiles
to a `MutatorTree<GfxMutator>` at apply time, with a
`SectionContext` for runtime value injection.

**What it's for.** Declaring mutators by hand as
`MutatorTree<GfxMutator>` is fine for Rust code but hostile to
JSON authoring. The builder DSL solves this: authors write
`MutatorNode` in JSON (the shape is nearly identical to
`GfxMutator` but serialisable and with `Repeat` for "N
consecutive channels with the same template"), and the builder
walks the AST with a `SectionContext` to resolve runtime values
(counts, fields, dynamically-chosen mutations) into a concrete
tree ready for `walk_tree_from`. This is the seam
[custom mutations](#4-the-mutation-framework) attach to.

**Under the hood.** `lib/baumhard/src/mutator_builder/`. The AST:
`MutatorNode::{Void, Single, Macro, Instruction, Repeat}`. The
indirection enums `ChannelSrc`, `CountSrc`, `MutationSrc` each
have a `Literal` variant (inline) and a `Runtime(String)` or
`SectionIndex` variant that consults the `SectionContext`
trait at build time. `build(ast, context)` returns a
`MutatorTree<GfxMutator>` with `Repeat` expanded to N children
on consecutive channels.

### Font system — `FONT_SYSTEM`, `AppFont`, `attrs_list_from_regions`

**Summary.** A single global cosmic-text `FontSystem`, a
compile-time enum of available fonts, and the one bridge
function from `ColorFontRegions` to cosmic-text's `AttrsList`.

**What it's for.** Every piece of text shaping in the project
flows through these three. `fonts::init()` is called once at
startup; the `FONT_SYSTEM` `RwLock` is acquired through
`acquire_font_system_write("site-name")` with a timeout-guarded
write lock. `AppFont` is generated at build time by scanning
`lib/baumhard/src/font/fonts/` — drop a font file in, recompile,
and the variant appears.

**Under the hood.** `lib/baumhard/src/font/`. The blessed entry
`attrs_list_from_regions` in `attrs.rs` is the only place
cosmic-text styling is constructed from Baumhard types.
Unknown fonts fall back to `Family::Monospace` with a
`log::warn!` rather than aborting — interactive paths must not
panic. The 5-second timeout on the write lock is a re-entrancy
bug detector: the single-threaded app should never wait on this
lock, so a timeout means the same thread is trying to acquire
twice.

### `RegionParams`, `RegionIndexer`, `RegionError`

**Summary.** A grid-bucketed spatial index over colour/font
regions for cheap hit-testing.

**What it's for.** Hit-testing "which region contains this point?"
against hundreds of spans over thousands of glyphs would be
linear per query. `RegionIndexer` divides the rendered surface
into a grid of buckets; queries consult the bucket containing
the point and scan only that bucket's regions. `RegionParams`
configures the grid, adapting to the resolution so dimensions
that don't factor cleanly (primes, near-primes) still get a
sensible subdivision.

**Under the hood.** `lib/baumhard/src/gfx_structs/util/`.
`RegionError::{Updating, InvalidParameters, Poisoned}` covers
the three failure modes; callers match and decide rather than
panicking. The indexer keeps in sync with the tree via the
`MutatorTree::apply_to` path — never mutate regions outside
that path, or the index drifts silently.

**Vision.** Per-tree spatial indexing is a seam
([`Tree::region_params` / `region_index` fields](#treet-m));
currently the index is scene-wide but the plumbing to push it
per-tree already exists.

### Animation primitives — `AnimationDef`, `AnimationInstance`, `Timeline`, `TimelineEvent`

**Summary.** An immutable animation blueprint (`AnimationDef`)
and a per-playback state struct (`AnimationInstance`) driven by
a `Timeline` of `TimelineEvent`s.

**What it's for.** Glyph animations need to define a sequence
once and replay it many times at different speeds, phases, or
counts without cloning the definition. `AnimationDef` is the
shared blueprint (via `Rc`); `AnimationInstance` carries the
live play state. The timeline is a list of events —
`Mutator(id)`, `Interpolation { mutator, num_frames,
duration }`, `WaitMillis(n)`, `Goto(idx)`, `Terminate` —
processed by the animation driver one event at a time.

**Under the hood.** `lib/baumhard/src/core/animation.rs`. A
`TimelineBuilder` provides a fluent constructor. The
`AnimationMutator` trait exists alongside `Mutator` so an
animation step can interpolate rather than apply instantly.

**Vision.** This is today's vocabulary for motion; the
`Followup` slot on mutation timing ([§4](#animation-timing))
expects to extend it with loop/reverse/chain semantics.

### Utilities — `grapheme_chad`, `color`, `geometry`

**Summary.** The shared-primitive toolkit: grapheme-aware text
operations, colour types and macros, and epsilon-aware 2D
geometry helpers.

**What it's for.** Three small modules that the rest of the
codebase builds on, rather than each module re-implementing its
own take:

- **`grapheme_chad`** — the only legitimate way to manipulate
  `String`/`&str` when the offset comes from user input.
  Functions: `count_grapheme_clusters`,
  `find_byte_index_of_grapheme`,
  `replace_graphemes_until_newline`, `split_off_graphemes`,
  `delete_back_unicode`, `delete_front_unicode`,
  `find_nth_line_grapheme_range`, `count_number_lines`. Byte
  slicing from user-facing counts lands mid-cluster on the
  first emoji; always reach for these. See
  [`CODE_CONVENTIONS.md §1`](./CODE_CONVENTIONS.md) and
  [`lib/baumhard/CONVENTIONS.md §B3`](./lib/baumhard/CONVENTIONS.md).
- **`color`** — `FloatRgba = [f32; 4]` and `Rgba = [u8; 4]`
  colour types, `Palette = Vec<FloatRgba>`, plus compile-time
  macros `rgb!`, `rgba!`, and (non-const) `hex!`. Channel-index
  constants for consistency.
- **`geometry`** — `almost_equal` (`|a - b| ≤ 1e-5`, the
  baumhard-wide epsilon), `clockwise_rotation_around_pivot`,
  y-dominant `pixel_greater_than` and siblings (cursor-reading
  order), `vec2_area`. `Comparator` float equality uses
  `almost_equal`.

**Under the hood.** `lib/baumhard/src/util/`. All pure functions,
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

**Summary.** The document root: nodes, edges, canvas configuration,
palettes, custom mutations.

**What it's for.** Everything a user can save and reload is here.
The `MindMap` is a plain serialisable struct — no derived state, no
runtime caches. The loader deserialises it from JSON; the
[scene builder](#scene-builder) and [tree builder](#tree-builder)
project it into renderable form; mutations transform it in place.
Helper methods (`children_of`, `all_descendants`,
`is_hidden_by_fold`, `is_ancestor_or_self`, `resolve_theme_colors`)
walk the data on demand rather than caching.

**Under the hood.**
`lib/baumhard/src/mindmap/model/mod.rs`. The shape is a flat
`HashMap<String, MindNode>` (keyed by Dewey id), a
`Vec<MindEdge>`, a [`canvas: Canvas`](#canvas), a `palettes:
HashMap<String, Palette>`, and
`custom_mutations: Vec<CustomMutation>`. See
[`format/schema.md`](./format/schema.md) for the JSON surface and
[`format/README.md`](./format/README.md) for a minimum-viable
example.

### `Canvas`

**Summary.** The per-map shared rendering context: background
colour, default node and connection styles, live theme-variable
map, named theme presets.

**What it's for.** Some things are per-map rather than per-node:
the canvas background colour, the defaults nodes and edges fall
back to when their fields are absent, the `var(--name)` theme
variables colours reference, and the presets theme-switching
mutations copy into those live variables. `Canvas` is that
shared state. It sits on `MindMap` directly (`canvas: Canvas`)
and is consulted at scene-build time for defaults and theme
resolution.

**Under the hood.**
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

**Summary.** One node — text, position, size, style, layout hint,
palette binding, channel, and trigger bindings.

**What it's for.** The unit of content. Each node renders as a
shape with text inside, optionally framed by a glyph border, and
participates in the parent-child tree through its `parent_id`.
Beyond the obvious display fields, it carries a `channel` for
mutation routing, a `color_schema` referencing a palette, optional
`text_runs` for rich text, optional `trigger_bindings` mapping
input to custom mutations, optional `inline_mutations` (the
lowest-precedence mutation source), and optional zoom-bounds.

**Under the hood.** `lib/baumhard/src/mindmap/model/node.rs`. Full
field reference in [`format/schema.md`](./format/schema.md). Author
owns non-overlap of node AABBs; the model does no collision
checking. The tree builder excludes folded subtrees from the
display tree; the underlying data persists either way.

### `MindEdge`

**Summary.** A directed connection between two nodes — line-mode
or portal-mode — with style, optional label, and optional
per-endpoint state.

**What it's for.** Edges carry both hierarchical structure (when
their `type` is `parent_child`) and arbitrary cross-links (when
`type` is `cross_link`). They render as either a path of glyphs
along a Bézier curve (line mode) or a pair of small markers, one
at each endpoint (portal mode). A line-mode edge can have a
single text label sitting along the path; a portal-mode edge has
two endpoint records, each with its own text and styling.

**Under the hood.** `lib/baumhard/src/mindmap/model/edge.rs`. Edges
have **no stable id** — they are identified by the tuple
`(from_id, to_id, edge_type)`
([`CODE_CONVENTIONS.md §3`](./CODE_CONVENTIONS.md)). The
`display_mode` field switches rendering style without changing the
underlying edge identity; flipping a long edge from line to portal
is a one-field change. Field reference in
[`format/schema.md`](./format/schema.md).

**Caveat.** Multiple edges between the same pair with different
`type` are allowed (rare but legitimate). Multiple edges with the
*same* tuple are a duplicate and a validation error.

### Dewey-decimal IDs

**Summary.** Dot-separated hierarchical node IDs (`"0"`, `"1.2"`,
`"1.2.3"`) that encode tree structure in the key itself.

**What it's for.** Reading a `.mindmap.json` reveals the tree shape
in the keys. IDs sort as numbers segment-by-segment (`"1.10"` after
`"1.9"`, not before), and `derive_parent_id` recovers the parent
without pointer chasing. The format is human-friendly and
diff-friendly — exactly the sort of place where opaque UUIDs would
have ended in the same byte count and zero readability.

**Under the hood.** `lib/baumhard/src/mindmap/model/mod.rs`.
`id_sort_key` extracts the last segment for sibling sort;
`derive_parent_id` strips it. Fresh IDs are minted by
`fresh_child_id` in `src/application/document/topology.rs`
without reusing deleted gaps. Full reference:
[`format/ids.md`](./format/ids.md).

**Caveat.** IDs do **not** cascade on runtime reparent — when
node `"1.2"` moves under `"0"`, it stays `"1.2"` and `parent_id`
becomes the truth. They *do* cascade on delete-with-orphan-promote.
This trade keeps reparent cheap; `maptool verify` flags drift.

### Channels (mindmap level)

**Summary.** The `MindNode.channel` field — the user-facing
surface of the Baumhard routing tag.

**What it's for.** Authors tag siblings with channels to opt them
into selective mutations. A `CustomMutation` whose mutator targets
channel 1 hits only siblings tagged 1; siblings tagged 0 are
skipped. Multiple siblings can share a channel (broadcast group),
or each can be unique (per-sibling targeting). All existing maps
default to channel 0 and behave as if the field did not exist.

**Under the hood.** Stored as `usize` on `MindNode`; preserved
through tree builder onto the corresponding `GfxElement.channel`;
consulted by [`BranchChannel`](#channel-and-branchchannel) at walk
time. Full reference:
[`format/channels.md`](./format/channels.md).

**Vision.** A `TargetScope::ChildrenOnChannel(n)` variant is the
named extension waiting on this field — it would let a mutation
declare "children whose channel is 1" without an inline predicate.

### Palettes

**Summary.** Map-level named colour schemes; nodes reference them
through `color_schema { palette, level, … }` rather than carrying
colours inline.

**What it's for.** The legacy miMind format stored full palette
data on every node; the testament map alone duplicated the same
~225 palettes across nodes. Hoisting palettes to the document
level is a 100× reduction in file size and turns "rethemes the
whole map" into a single edit. Each palette is an array of
`ColorGroup`s indexed by depth; a node's `level` is which group
it pulls from. Level-clamping (last group when out of range) makes
deep subtrees degrade gracefully.

**Under the hood.**
`lib/baumhard/src/mindmap/model/palette.rs`. A node's binding
lives in its optional `color_schema` field, a `ColorSchema`
record with `palette: String` (the key into `map.palettes`),
`level: usize` (which `ColorGroup` to pull from), and two
flags — `starts_at_root` (does level 0 apply to the schema
root or to its children?) and `connections_colored` (do edges
inherit the palette stroke colour?). `resolve_theme_colors` on
`MindMap` does the lookup; out-of-range `level` clamps to the
last group rather than failing. Validation requires every
referenced palette to exist with at least one group. Full
reference: [`format/palettes.md`](./format/palettes.md).

**Vision.** Animated palette transitions are the seam — the data
shape is already mutation-friendly; the runtime would need to
interpolate `ColorGroup` fields on a clock.

### Text runs

**Summary.** Non-overlapping styled character ranges within a
node's text — bold, italic, underline, font, size, colour,
hyperlink.

**What it's for.** A single node can have rich text without being
fragmented into multiple nodes. Text runs are the mindmap-side
surface that the renderer translates into `ColorFontRegions`
spans for shaping. The user-visible effect is a per-span
override: emphasis on the first word, a coloured annotation in
the middle, a link at the end — all on one node.

**Under the hood.** `lib/baumhard/src/mindmap/model/node.rs`.
Each run carries `start`, `end`, `bold`, `italic`, `underline`,
optional `font`, optional `size_pt`, optional `color`, optional
`hyperlink`. Indexed by **Unicode code points**, not bytes and
not graphemes — this matches `ColorFontRegions` and the legacy
miMind format. Indices are stable across round-trip even when
text contains characters outside the BMP (more bytes than code
points) or combining marks (more code points than graphemes).
Validation: non-overlapping, ascending, `end <= text's
code-point count`. Uncovered ranges inherit the node-level
style. Full reference:
[`format/text-runs.md`](./format/text-runs.md).

**Caveat.** If `text_runs` is non-empty, **only covered ranges
render** — uncovered graphemes drop silently. So authors must
cover every grapheme they want visible, not just the ones they
want to restyle. This is by design (it simplifies the
renderer's region pass) but it is the single biggest trap in
the format; `maptool verify` does not catch partial-coverage
intent vs. accident.

### Theme variables

**Summary.** Document-level CSS-style named colours referenced as
`var(--name)` from any colour field.

**What it's for.** Avoids hex repetition across hundreds of nodes
and edges. A theme switch changes the variable; everything
referencing it updates. Theme variants (presets) can be stored
under `canvas.theme_variants` and applied through the
`SetThemeVariant` document action.

**Under the hood.** Resolved at scene-build time in the colour
cascade — variable lookup, then fall through to a default if the
name is unknown. Document actions
[`SetThemeVariant`](#document-actions) and `SetThemeVariables`
mutate the live `canvas.theme_variables` map.

### Zoom bounds

**Summary.** The mindmap-level surface of
[`ZoomVisibility`](#zoomvisibility): two flat fields
(`min_zoom_to_render`, `max_zoom_to_render`) on every renderable
entity.

**What it's for.** Authors reach for zoom bounds to tier detail by
zoom: a label fades in past 1.5×, a portal endpoint fades out
below 0.5×. The field appears on `MindNode`, `MindEdge`,
`EdgeLabelConfig`, and `PortalEndpointState`.

**Under the hood.** **Replace, not intersect** cascade: when a
label or portal endpoint declares either bound, it replaces the
parent edge's window entirely. Intersection would silently
inherit a bound the author did not mention. Full reference:
[`format/zoom-bounds.md`](./format/zoom-bounds.md). Authoring via
the `zoom` console verb (`zoom min=1.5 max=3.0`, `zoom clear`,
`zoom max=unset`) is wired against the active selection.

### Border geometry

**Summary.** Glyph-drawn frames around nodes — Unicode box-drawing
characters laid out around the node's AABB.

**What it's for.** Borders are the visual frame that gives a node
its "boxed" appearance. They are made of glyphs (light, heavy,
double, rounded, or fully custom box-drawing chars), not solid
strokes — consistent with the
[everything-is-glyphs](#everything-is-glyphs) invariant. Borders
also serve as anchor surfaces for portal endpoints, which sit at
parametric positions along the border perimeter.

**Under the hood.** `lib/baumhard/src/mindmap/border.rs`. The
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

**Caveat.** Borders today only render on rectangular nodes
(`NodeShape::Rectangle` and `style.show_frame = true`). Ellipse
borders need shape-aware glyph layout — a named seam in
[§8](#8-named-trajectory--vision).

### `GlyphConnectionConfig`

**Summary.** The per-edge rendering configuration: body glyph,
caps, font, font size, screen-space font clamps, color.

**What it's for.** Every `MindEdge` carries one. `GlyphBorderConfig`
is to a node what `GlyphConnectionConfig` is to an edge: the
shape of the glyphs that draw the thing. The body glyph is
repeated along the connection path; `cap_start` and `cap_end`
override the terminal glyphs if present. Font size is
interpreted as the target *on-screen* size at zoom = 1.0;
`min_font_size_pt` and `max_font_size_pt` clamp the effective
screen-space size as the camera zooms, so a long edge stays
readable both zoomed in and zoomed out.

**Under the hood.** `lib/baumhard/src/mindmap/model/edge.rs:335+`.
Fields: `body: String` (default mid-dot `·`), `cap_start` /
`cap_end: Option<String>`, `font: Option<String>`, `font_size_pt:
f32`, `min_font_size_pt` / `max_font_size_pt: Option<f32>`,
`color: Option<String>`. Colour cascade priority (highest
first): edge-label → `glyph_connection.color` → `edge.color`.
`effective_font_size_pt(zoom)` is the helper callers reach for
to derive the clamped screen-space size.

### `ControlPoint`

**Summary.** An author-set Bézier offset on a `MindEdge`,
expressed as an offset from a node centre rather than an
absolute canvas coordinate.

**What it's for.** Straight line-mode edges can become curved
when the author specifies control points. Zero control points
is a straight segment; one promotes to a cubic Bézier (via
quadratic-to-cubic lifting); two or more define a cubic
directly. Control points live as offsets from endpoint centres
so a node move drags the curve along without the author
having to re-tune the path.

**Under the hood.**
`lib/baumhard/src/mindmap/model/edge.rs`. Consumed by
[connection path construction](#connection-paths), where
`build_connection_path` converts control points from offsets
into cubic control coordinates in canvas space.

### Portals

**Summary.** Edges with `display_mode = "portal"`: rendered as two
glyph markers, one at each endpoint, instead of a connecting
line.

**What it's for.** When two endpoints are far apart on the canvas,
drawing a literal line between them is visually noisy and
expensive (hundreds of glyphs). Portals decouple the visual link
from the physical span: the user sees a small glyph at each end,
recognises them as a pair (matching colour, matching text), and
can double-click either to fly the camera to the partner.
Portals share the underlying edge with line-mode — the only
difference is `display_mode`.

**Under the hood.**
`lib/baumhard/src/mindmap/model/edge.rs`. Per-endpoint state lives
in `PortalEndpointState`: `color`, `border_t` (parametric
position on the owning node's border), `perpendicular_offset`
(signed distance along the outward normal), `text`, `text_color`,
`text_font_size_pt`, `text_min/max_font_size_pt`,
`min/max_zoom_to_render`. The icon and the adjacent text are
separate hitboxes (`portal_icon_hitboxes` /
`portal_text_hitboxes` in the renderer), so a click on the icon
selects `SelectionState::PortalLabel` (and font/colour ops target
the icon channel) while a click on the text selects
`SelectionState::PortalText` (and ops target the text channel).
Full reference: [`format/portal-labels.md`](./format/portal-labels.md).

### Edge labels

**Summary.** Optional text along a line-mode edge, positioned by
parametric `t` along the path with an optional perpendicular
offset.

**What it's for.** Edge annotations — "depends on", "blocks",
"derived from". Line-mode only; portal edges use per-endpoint
text instead. Labels can be dragged to reposition (native today),
authored via the `label position_t=… perpendicular=…` console
verb (cross-platform), and given their own zoom-window override.

**Under the hood.** `EdgeLabelConfig` on `MindEdge`. Position
encoded as `(position_t, perpendicular_offset)`. Drag computes
both via `closest_point_on_path`. Replace-not-intersect zoom
cascade matches portals.

### Connection paths

**Summary.** The geometric backbone of edge rendering: straight
segments and cubic Bézier curves with anchor resolution at the
endpoints.

**What it's for.** Given two node AABBs and optional control
points, compute the curve along which to lay out edge glyphs.
The same path math powers glyph placement (sample at uniform arc
length), label drag (project cursor onto path), and hit-testing
(distance from cursor to path).

**Under the hood.** `lib/baumhard/src/mindmap/connection/`. Key
functions: `build_connection_path` (from anchors + control
points), `resolve_anchor_point` (auto / top / right / bottom /
left), `point_at_t`, `tangent_at_t`, `closest_point_on_path`
(uniform-t sampling + Newton refinement for cubics, direct
projection for straight lines), `sample_path` (arc-length-uniform
glyph placement), `distance_to_path`. Quadratics get promoted to
cubic at build time, so the apply path is always one of two
shapes.

### Portal geometry

**Summary.** The conversion between `border_t ∈ [0, 4)` and a
canvas point on a rectangular node's border, plus directional
defaults.

**What it's for.** Portal endpoints must sit on their owning
node's border, parametrically — so when the node is resized, a
label at "the middle of the right edge" stays at the middle of
the right edge. The side-indexed encoding (`[0, 1)` = top,
`[1, 2)` = right, `[2, 3)` = bottom, `[3, 4)` = left) is the right
abstraction: stable across resize, deterministic across corners.

**Under the hood.** `lib/baumhard/src/mindmap/portal_geometry.rs`.
Functions: `wrap_border_t` (rem-Euclid into `[0, 4)`),
`border_point_at`, `border_outward_normal`,
`default_border_t` (the auto-orientation: cast a ray from owner
to partner centre), `nearest_border_t` (project a canvas point to
the closest border parameter, used by drag-snap).

### Fold state

**Summary.** A boolean per node; folded subtrees are excluded
from the display tree but persist in the model.

**What it's for.** Hide subtrees without losing data. The user
can collapse a region of the map; reopening restores it. The
scene builder and tree builder both consult
`MindMap::is_hidden_by_fold`, which walks the parent chain.

**Under the hood.** `MindNode.folded: bool`; the cascading
visibility check is `O(depth)` per node and runs once per scene
build.

### Tree builder

**Summary.** Projects a `MindMap` into a Baumhard
`Tree<GfxElement, GfxMutator>` mirroring the parent-child
structure, with sibling order
`(channel, id_sort_key)`.

**What it's for.** Mutations need a `Tree` to walk against. The
tree builder constructs it from the model: each visible
`MindNode` becomes a `GfxElement::GlyphArea`, parent-child
relationships become tree edges, channels are preserved. Per-role
sub-builders cover borders, portals, connections, edge labels,
and edge handles, each producing its own tree (and matching
mutator-tree) so per-role mutations stay scoped.

**Under the hood.**
`lib/baumhard/src/mindmap/tree_builder/mod.rs`. Returns a
`MindMapTree` with `node_map: HashMap<String, NodeId>` and the
reverse map for cheap lookups in either direction. Folded nodes
are excluded.

### Scene builder

**Summary.** Projects a `MindMap` into a flat `RenderScene` of
plain-data elements (text, borders, connections, portals,
labels, handles) for direct GPU consumption.

**What it's for.** The renderer wants flat element lists, not a
tree. The scene builder walks the model, applies style cascades
(theme variables, palette resolution, zoom windows, transient
edit previews), samples connection paths, and emits a transient
`RenderScene` rebuilt every frame. Caching at the
[`scene_cache`](#scene-cache) level reuses sampled positions when
endpoints don't move.

**Under the hood.** `lib/baumhard/src/mindmap/scene_builder/`.
Per-role modules: `node_pass` (text + borders + clip AABBs),
`connection`, `label`, `portal`, `edge_handle`. Output is the
`RenderScene` struct, whose fields are plain-data element
lists:

- `text_elements: Vec<TextElement>` — node text, position,
  size, style runs.
- `border_elements: Vec<BorderElement>` — glyph-drawn frames
  around nodes, including zoom visibility.
- `connection_elements: Vec<ConnectionElement>` — sampled
  glyph positions along each line-mode edge, with body/cap
  glyphs and colour.
- `portal_elements: Vec<PortalElement>` — per-endpoint glyph
  markers for portal-mode edges.
- `connection_label_elements: Vec<ConnectionLabelElement>` —
  positioned text labels along connection paths.
- `edge_handles: Vec<EdgeHandleElement>` — grab-handle glyphs
  on the selected edge (anchors, control points, midpoint).
- `background_color: String` — canvas fill colour.

These element structs are the intermediate representation that
sits between the Baumhard tree side of the pipeline and the GPU
commands the renderer actually submits. Selection highlight is
applied at emission time, not stored on the model; drag-preview
offsets and color-picker previews are read from the document
but never committed back.

### Scene cache

**Summary.** A per-edge cache of sampled glyph positions, keyed
on `(from_id, to_id, edge_type)`, reused across frames when
endpoints have not moved.

**What it's for.** Sampling a cubic Bézier path at uniform arc
length is the most expensive per-edge work. The cache invalidates
on endpoint drag (via `drag_offsets`) and on zoom or structural
change. Otherwise the previous frame's samples are reused with a
cheap `point_inside_any_node` clip filter.

**Under the hood.** `lib/baumhard/src/mindmap/scene_cache.rs`.

### Trigger bindings

**Summary.** Per-node bindings of input events to custom-mutation
ids: `OnClick`, `OnHover`, `OnKey`, `OnLink`.

**What it's for.** Authoring interactive map elements without
custom code: a button-like node fires a custom mutation when
clicked. Bindings carry an optional context filter (Desktop /
Web / Touch); empty means "all platforms".

**Under the hood.** `MindNode.trigger_bindings`; dispatch lives
in the application's input handlers. Missing mutation IDs are
silent no-ops — runtime ignores rather than panicking.

---

## §4 The mutation framework

The mutation framework is the primary extensibility seam. It spans
both crates — the AST and walker live in Baumhard, the registry
and dispatch live in Mandala — and is the answer to "how do I add
behaviour to a mindmap without recompiling?" Everything from
"grow font 2pt on the selected subtree" to size-aware layouts like
`flower-layout` and `tree-cascade` flows through it.

For the JSON authoring surface see
[`format/mutations.md`](./format/mutations.md). For the
prescriptive carrier shape see
[`lib/baumhard/src/mindmap/custom_mutation/`](./lib/baumhard/src/mindmap/custom_mutation/).

### `CustomMutation`

**Summary.** The carrier struct: an id, name, description,
contexts, optional mutator AST, target scope, behavior, optional
document actions, and optional animation timing.

**What it's for.** A `CustomMutation` is one named, reusable
operation. Authored as JSON (declarative) or registered in Rust
(imperative); referenced by id from console verbs, trigger
bindings, or other custom mutations. The same shape covers tiny
deltas ("add 2pt to font") and structural algorithms
(`flower-layout`).

**Under the hood.**
`lib/baumhard/src/mindmap/custom_mutation/mod.rs`. Fields: `id`
(unique key), `name` and `description` (human-readable),
`contexts` (taxonomy tags — see
[contexts](#contexts-taxonomy)), `mutator` (an optional
`MutatorNode` AST), `target_scope` (which nodes the change
covers), `behavior` (`Persistent` or `Toggle`),
`document_actions` (optional canvas-level operations), `timing`
(optional `AnimationTiming`).

### Four-source loader

**Summary.** Mutations merge from four sources at startup, with
ascending precedence: App < User < Map < Inline.

**What it's for.** Authors at every layer can define mutations
without stepping on each other. A bundled "grow-font-2pt" can be
overridden by the user's personal version, which can in turn be
overridden by a map's local definition, which can in turn be
overridden by a single node's `inline_mutations`. The
`MindMapDocument::mutation_sources` map records which layer won
each id, so `mutation help <id>` can report it.

**Under the hood.**
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
`MindMapDocument::mutation_sources` as a `MutationSource`
enum (`App` / `User` / `Map` / `Inline`), so
`mutation help <id>` on the console can report which layer
won a given id.

### Declarative path — `MutatorNode` AST

**Summary.** A pure-data AST that compiles to a
`MutatorTree<GfxMutator>` and runs through the standard tree
walker.

**What it's for.** Any mutation expressible as a tree of
field-level deltas with control-flow instructions belongs here.
This is the default: write JSON, the runtime walks it. The AST
shape mirrors `GfxMutator` — `Void`, `Single`, `Macro`,
`Instruction` — plus a `Repeat` wrapper for "N consecutive
children at consecutive channels with the same template" (the
flower-petal pattern, etc.).

**Under the hood.**
`build_mutator(ast, context)` in `lib/baumhard/src/mutator_builder/`
walks the AST recursively, expands `Repeat` to N children with
incrementing channels, resolves `Runtime("<label>")` holes via the
`SectionContext`, and returns a fully-inflated `MutatorTree`. The
walker then applies it via `apply_to`. After application, changed
elements are synced back to the model so the change persists across
the next scene build.

### Imperative path — `DynamicMutationHandler`

**Summary.** A registered Rust function pointer the dispatcher
calls directly when the AST is too narrow.

**What it's for.** Some operations are inherently imperative —
arbitrary BFS layouts, multi-pass spatial algorithms, anything
that needs runtime control flow the walker doesn't provide. The
handler registry lets them live as Rust functions registered at
startup, with the same `id`/contexts/target-scope surface as a
declarative mutation.

**Under the hood.**
`src/application/document/mutations/`. Built-in handlers:
- `flower_layout.rs` — radial child arrangement.
- `tree_cascade.rs` — hierarchical cascading layout.

`register_builtin_handlers()` wires them at startup. Adding a new
handler is: new module, new function, new registration call, new
matching id in `assets/mutations/application.json`.

**Caveat.** When a higher-precedence layer (User / Map / Inline)
declares the same id as a registered handler, **the declarative
mutator wins**. The handler is bypassed. This prevents a subtle
hijack where a user's JSON would silently invoke imperative code
they did not author.

### Target scopes

**Summary.** Six variants telling the dispatcher which nodes the
mutation covers — also used as the snapshot window for undo.

**What it's for.** A mutation declares "I touch this node only" or
"I touch this node and all its descendants" and the dispatcher
both walks the right subtree and snapshots the right set for
undo. The undo-snapshot equivalence is the load-bearing detail:
if a mutation's `target_scope` is too narrow, undo will not
fully reverse it.

**Under the hood.** Variants: `SelfOnly`, `Children`,
`Descendants` (not the anchor), `SelfAndDescendants`, `Parent`,
`Siblings` (the anchor's siblings, excluding itself). Scope
helpers in `custom_mutation::scope` produce matching
`MutatorNode` shapes for the AST walker.

### Behaviors — `Persistent` vs. `Toggle`

**Summary.** Whether the mutation commits to the model and pushes
an undo entry (`Persistent`) or only modifies the display tree
and remembers itself in `active_toggles` (`Toggle`).

**What it's for.** Some mutations are "apply and remember"
(persistent — visual change, undo coverage); others are
"reversible inspection" (toggle — visual change without model
commit, second trigger reverses). Toggles are the right shape
for "highlight this", "expand this preview", "show debug
overlay".

**Under the hood.** Persistent: snapshot affected nodes, apply,
sync back, push undo. Toggle: apply to tree only, insert
`(node_id, mutation_id)` into `MindMapDocument::active_toggles`;
on second trigger from the same anchor, remove the pair (undo
stack gets no entry — re-triggering is the reverse).

### Contexts taxonomy

**Summary.** Dotted-namespace tags describing what a mutation
operates on: `"internal"`, `"map"`, `"map.node"`, `"map.tree"`,
plus the reserved `"plugin.<name>.<kind>"` namespace.

**What it's for.** The console's `mutation list` filters by
context so users see only mutations relevant to their current
selection. `"internal"` hides a mutation from listing entirely
(used by handlers that compose into other mutations). The
plugin namespace is the home of future plugin-authored
mutations.

**Under the hood.** `matches_context(query)` returns true if the
mutation's `contexts` include `query` exactly or sit inside its
dotted prefix; `matches_context("map")` hits both `"map.node"`
and `"map.tree"`.

### Document actions

**Summary.** Canvas-level operations a mutation can carry
alongside (or instead of) its tree mutations:
`SetThemeVariant(name)`, `SetThemeVariables(map)`.

**What it's for.** "Switch the theme" is not a per-node delta —
it touches `canvas.theme_variables`. Document actions cover that
seam. They run alongside the tree mutation; a single mutation
can both restyle nodes and switch the theme in one apply.

**Under the hood.**
`lib/baumhard/src/mindmap/custom_mutation/document_action.rs`.
`SetThemeVariant` copies a named preset from
`canvas.theme_variants` into the live `theme_variables`;
`SetThemeVariables` overwrites individual entries while
preserving unmentioned keys.

### Animation timing

**Summary.** Optional duration / delay / easing wrapper around
any mutation, turning instant application into a clock-driven
interpolation.

**What it's for.** A "grow font" that snaps is fine; one that
animates over 300ms reads better. The timing wrapper lets a
declarative mutation carry that timing without authoring an
animation by hand. The dispatcher starts an `AnimationInstance`
that ticks each frame, blends the in-flight state, and commits
on completion.

**Under the hood.**
`lib/baumhard/src/mindmap/custom_mutation/timing.rs`. Fields:
`duration_ms`, `delay_ms`, `easing` (`Linear` / `EaseIn` /
`EaseOut` / `EaseInOut`), and a reserved `then` (`Followup`)
slot.

**Vision.** `Followup::{Reverse, Chain, Loop}` is named but not
yet wired. When it lands, mutations will compose into chains
and oscillations without scripting.

### Runtime holes — `SectionContext`

**Summary.** A trait the host implements to feed runtime values
into a `MutatorNode` AST at build time.

**What it's for.** Some mutations need values the AST can't
inline — the count of currently-visible children, the cursor
position when invoked, a field looked up from the selected
node. `MutationSrc::Runtime("<label>")` and
`CountSrc::Runtime("<label>")` defer those holes to a
`SectionContext` registered per-mutation-id; the builder consults
it as it walks. Pure-data mutations (no holes) use a no-op
context.

**Under the hood.**
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

**Summary.** The native event-loop entry points. `Application` is
the pre-window root; `InitState` is the persistent post-window
state; `NativeApp` is the winit `ApplicationHandler` glue.

**What it's for.** The platform separation is honest: pre-window
work (parse args, init fonts, load mutations) happens before any
GPU resources exist; once the OS gives us a window, we transition
to `InitState` and stay there for the lifetime of the run.
`NativeApp` exists only to satisfy winit's trait surface;
everything substantive lives on `InitState`.

**Under the hood.** `src/application/app/run_native.rs:48-130`.
`InitState` carries `window: Arc<Window>`, an optional
`document: Option<MindMapDocument>` (`None` before first file
load), `drag_state`, `app_mode`, modal UI state (console,
text/label/portal-text editors, color picker), `picker_hover`,
and the resolved keybind table. The `input_context()` method at
line 137 produces a borrowed view of these fields per-event so
handlers can borrow disjoint subsets without lifetime
contortions.

### Event loop and `drain_frame`

**Summary.** The per-frame heartbeat: tick watchdog, drive
throttled interactions, advance animations, rebuild geometry,
rebuild scene if dirty, render, log frame interval.

**What it's for.** Every frame runs the same six steps in the
same order. Inputs arriving between frames mutate the document
and set the `dirty` flag; the next `drain_frame` consults the
flag and rebuilds only what changed. This decouples mutation
frequency (often per-input-sample) from rebuild frequency
(at most once per frame), so a flurry of pointer events doesn't
trigger a flurry of scene rebuilds.

**Under the hood.** `src/application/app/drain_frame.rs`. Called
on every winit `AboutToWait` event. Step order:

1. Drive any active throttled interaction
   ([`ThrottledInteraction`](#throttledinteraction-and-throttleddrag)) —
   apply pending delta if the throttle says drain.
2. Advance running animations; on completion, push undo entry.
3. Rebuild connection geometry if edges moved.
4. If `dirty` and no modal editor is open, schedule a scene
   rebuild.
5. Dispatch to `Renderer::process` to push GPU buffers.
6. Update FPS rolling-average / snapshot counter.

### `MindMapDocument`

**Summary.** The data plane: owns the `MindMap`, the tree mirror,
the undo stack, the running animations, and the mutation
registries.

**What it's for.** This is where every persistent piece of state
lives. It is the only owner of the model and the undo stack; the
renderer reads from it, never mutates. The dirty flag belongs to
it. Transient previews (live colour picker, in-flight label edit,
in-flight portal-text edit) belong to it too — read by the scene
builder, never committed back without an explicit step.

**Under the hood.**
`src/application/document/mod.rs:64-151`. Fields include
`mindmap: MindMap`, `tree: Option<MindMapTree>`, `selection:
SelectionState`, `undo_stack: Vec<UndoAction>`,
`active_animations`, `active_toggles`, `mutation_registry`,
`mutation_handlers`, `mutation_sources`, `dirty: bool`,
`label_edit_preview`, `portal_text_edit_preview`,
`color_picker_preview`. What it does **not** own: the renderer,
GPU resources, drag/mode state, modal editor state, keybinds —
those are all on `InitState`.

### `SelectionState`

**Summary.** A tagged union of what the user has selected: nothing,
a node, multiple nodes, an edge body, an edge label, a portal
icon, or a portal text.

**What it's for.** Selection variants are mutually exclusive by
construction — at most one thing is selected at a time. The
variant tag is the routing key for everything operating on the
selection: which clipboard channel a copy goes through, which
colour field a colour command sets, which font field a font
command sets. The renderer uses it to apply the cyan highlight
to the right element.

**Under the hood.**
`src/application/document/types.rs:99-148`. Variants:

- `None`
- `Single(node_id)` — one node
- `Multi(Vec<node_id>)` — multiple nodes
- `Edge(EdgeRef)` — the whole edge body
- `EdgeLabel(EdgeLabelSel)` — the text label of a line-mode edge
- `PortalLabel(PortalLabelSel)` — a portal endpoint icon
- `PortalText(PortalLabelSel)` — a portal endpoint text

The four edge-adjacent variants (`Edge`, `EdgeLabel`,
`PortalLabel`, `PortalText`) each route to a different
clipboard / colour / font channel: copy on a `PortalLabel` reads
the icon colour; copy on a `PortalText` reads the text colour;
font commands write to the corresponding field group.

### `EdgeRef`

**Summary.** The `(from_id, to_id, edge_type)` triple that
identifies an edge.

**What it's for.** Edges have no stable id (§3:
[`MindEdge`](#mindedge)), so selection, undo entries, and
console arguments all carry this triple. Equality and lookup are
by triple match against the model's `Vec<MindEdge>`.

**Under the hood.**
`src/application/document/types.rs:71-97`. The `matches`
method walks the edge vector linearly; this is fine because
edges are sparse and the lookup happens at user-event frequency,
not in hot loops.

### `AppMode`

**Summary.** The transient modal state for reparent and connect
operations: `Normal` / `Reparent { sources }` / `Connect {
source }`.

**What it's for.** Some user actions take two clicks: select a
source, then click a target. Modes encode the in-between state.
Pressing Ctrl+R on a selection enters `Reparent`; the next click
on a node attaches the sources as its last children, and Esc
cancels. Pressing Ctrl+D on one node enters `Connect`; the next
click on another node creates a `cross_link` edge.

**Under the hood.** `src/application/app/mod.rs:328-338`.
Native-only today — both modes are gated `#[cfg(not(target_arch
= "wasm32"))]`. The mode is stored on `InitState` and consulted
by the click handler.

### `DragState`

**Summary.** The drag state machine: `None` / `Pending` /
`Panning` / `SelectingRect` / `Throttled(ThrottledDrag)`.

**What it's for.** Mouse-down does not commit to a drag yet —
the user might be clicking, or might be about to drag. `Pending`
captures everything the cursor was over at button-down; once
movement crosses the drag threshold, the state transitions to
`Panning` (empty space), `SelectingRect` (Shift+drag on empty
space), or one of the four `ThrottledDrag` variants depending
on what was hit.

**Under the hood.** `src/application/app/mod.rs:358-411`.
Native-only today. Hit priority on `Pending` is fixed: edge
handle > portal label > edge label > node, so small grab-areas
always win over larger AABBs.

### `ThrottledInteraction` and `ThrottledDrag`

**Summary.** A trait + four-variant enum providing one uniform
shell for continuous, high-rate-input drag types.

**What it's for.** Dragging a node, an edge handle, a portal
label, and an edge label all follow the same per-frame pattern:
accumulate input deltas, ask the throttle whether to drain,
apply if drain, otherwise wait. The trait factors that
accept-and-drain dance into one place; new throttled drags
attach as one struct + one trait impl + one enum variant
without growing the dispatch.

**Under the hood.**
`src/application/app/throttled_interaction/mod.rs:78-125`.
Trait methods: `has_pending`, `throttle`, `drain(ctx)`, `reset`.
Variants:

- `MovingNode(MovingNodeInteraction)`
- `EdgeHandle(EdgeHandleInteraction)`
- `PortalLabel(PortalLabelInteraction)`
- `EdgeLabel(EdgeLabelInteraction)`

`as_dyn_mut()` widens to `&mut dyn ThrottledInteraction` so the
drain dispatcher does not need to know each kind.

**Vision.** Touch gestures are the next obvious user — pinch
zoom, two-finger pan, long-press selection — each a new
`ThrottledDrag` variant with the same shape.

### `MutationFrequencyThrottle` (and `frame_throttle`)

**Summary.** An adaptive frame-counter throttle that gates
*application* of mutations under load while leaving *acceptance*
of input untouched.

**What it's for.** When per-frame work threatens the GPU
budget, the system must degrade gracefully. The non-negotiable
rule is **responsiveness is never traded for fidelity**: the
cursor must stay current with the hardware pointer at all
times, even if the dragged node updates only every fourth
frame. The throttle samples actual work duration into a moving
average; if the average exceeds budget, it raises `n` (the
"drain divisor"); if work is well under budget with hysteresis
margin, it lowers `n` toward 1.

**Under the hood.** `src/application/frame_throttle.rs:64-183`.
Default budget `14_000` µs (60 Hz minus safety), default
window 8 frames, default hysteresis 30%. `n` clamps in
`[1, 8]`. Each `ThrottledDrag` owns its own throttle, so
per-gesture profiles tune independently — a 500-node move
budget does not bias an edge-label drag's average.

### `UndoAction`

**Summary.** A 12-variant tagged union; one variant per
user-facing mutation, dispatched through `MindMapDocument::undo`
to reverse it.

**What it's for.** Every persistent change pushes one
`UndoAction`; Ctrl+Z pops the back of the stack and dispatches.
The discipline is **one mutation, one variant** — adding a new
mutation means adding a new variant, snapshotting the right
"before" state, and writing the matching `undo()` arm in the
same commit.

**Under the hood.**
`src/application/document/undo_action.rs:10-88`. The twelve
variants: `MoveNodes`, `CustomMutation`, `ReparentNodes`,
`DeleteEdge`, `CreateEdge`, `EditEdge`, `CreateNode`,
`EditNodeText`, `EditNodeStyle`, `EditNodeZoom`,
`CanvasSnapshot`, `DeleteNode`. `CustomMutation` is the general
bucket — it snapshots the `target_scope`-defined window so any
declarative or imperative mutation replays cleanly. Every arm is
bounds-checked (e.g. `index < edges.len()`) before mutating, so
undo is always safe — never panics, even on a partially-deleted
state.

### `Renderer`

**Summary.** The GPU resource holder and command-buffer builder;
reads from the document, writes to the swapchain.

**What it's for.** The `Renderer` is the view side of the
model/view split. It owns wgpu device, queue, surface,
pipelines, atlases, and the FPS ring buffer; every frame, its
`process()` reads document and scene state, builds command
buffers, and submits to the GPU. It never holds a reference to
the document.

**Under the hood.** `src/application/renderer/mod.rs:224-878`.
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

**Summary.** A two-role scene container: a camera-transformed
canvas and a screen-space overlay, each composed of named
sub-trees.

**What it's for.** Mindmap content (nodes, borders, connections,
portals, edge handles) belongs in the canvas role — pans and
zooms with the camera. The console and color picker belong in
the overlay role — fixed in screen space. The `AppScene`
abstracts that split; rebuild dispatch
(`InPlaceMutator` for small mutator-able changes,
`FullRebuild` for structural changes) flows through the same
seam for both roles.

**Under the hood.** `src/application/scene_host.rs:1-150`. Each
role has named slots (`CanvasRole`, `OverlayRole`); each slot
has a corresponding `Tree<GfxElement, GfxMutator>` and a
mutator registry. The same idiom drives both canvas-role
rebuilds (in `scene_rebuild.rs`) and overlay-role rebuilds
(console text changes, color picker re-layout).

### Scene rebuild granularity

**Summary.** Five tiered rebuild functions, each scoped to a
specific change kind.

**What it's for.** Different changes invalidate different
amounts of work. Editing a node's text might change its width
(full rebuild); dragging a node only moves connection paths
(connection-only rebuild); changing a portal endpoint colour
only touches portal markers (portal-only rebuild). Each tier
is dispatched explicitly so the cheapest one runs.

**Under the hood.** `src/application/app/scene_rebuild.rs`.
Functions: `rebuild_all` (full tree + scene), `rebuild_scene_only`
(reuse tree, rebuild scene), `update_connection_tree` (edges
only), `update_portal_tree` (portals only),
`update_border_tree_static` (borders only).

### Dirty flag

**Summary.** A single `bool` on `MindMapDocument` set by
mutations and consulted by `drain_frame`.

**What it's for.** Decouples mutation frequency from rebuild
frequency. A drag handler that fires 200 mutations per second
does not trigger 200 rebuilds; it sets `dirty = true` once and
the next frame's drain rebuilds once. After rebuild, the flag
resets.

**Under the hood.** Read at the top of `drain_frame`'s rebuild
step; reset at the bottom. Modal editors check it implicitly —
if a text-edit modal is open, the rebuild step is suppressed
because edits are visualised through the in-flight preview, not
the model.

### FPS overlay

**Summary.** Two display modes for frame-time diagnostics:
snapshot (stable readout, re-sampled periodically) and debug
(live rolling average).

**What it's for.** Performance-conscious development needs a
truthful FPS readout. The snapshot mode answers "what is the
steady-state frame rate?"; the debug mode answers "where are
the hitches?". The `fps` console verb (native) toggles between
them.

**Under the hood.** Embedded in
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

**Summary.** A native-only background thread that reads an
atomic timestamp pinged by the main loop and aborts the
process with a diagnostic banner if the main loop stalls past
threshold.

**What it's for.** Mandala is single-threaded; an infinite
loop, a same-thread `RwLock` re-entry, or a blocking GPU call
would hang indefinitely with no actionable error. The watchdog
turns a hang into a fast, diagnostic crash. It is the **only**
sanctioned background thread in the project — the
single-threaded invariant for the model/view pipeline is
preserved because the watchdog only *reads* a shared
`AtomicU64`, never touching app state.

**Under the hood.**
`src/application/app/freeze_watchdog.rs:38-134`. Main thread
calls `tick()` at every event-loop boundary; watchdog reads
the atomic every second; if the gap exceeds `FREEZE_THRESHOLD`
(10 seconds), prints diagnostics and aborts. Not present on
WASM — browsers already provide an "unresponsive tab" dialog
for free.

### `now_ms()`

**Summary.** A cross-platform monotonic clock returning `f64`
milliseconds since process start (native) or page load (WASM).

**What it's for.** Animation timing, double-click detection,
FPS tracking, throttled-interaction frame stamping all need a
clock that works the same on both targets. `now_ms()` is the
single bridge.

**Under the hood.** `src/application/app/mod.rs:98-111`.
Native: `Instant::now()` deltas from a static epoch. WASM:
`window.performance.now()` (clamped to ≥1ms by Spectre
mitigations).

---

## §6 The authoring surface

Authoring surface concepts are the parts a user actually touches:
modal editors, the console, keybinds, the color picker, clipboard,
and (briefly) the `maptool` CLI. Most are native-only today; the
parity story for each is honest.

### Inline node-text editor

**Summary.** Multi-line, grapheme-aware text editing on a
selected node; commit-on-click-outside, cancel on Esc.

**What it's for.** Editing a node's text without leaving the
canvas. Double-click or Enter opens the editor; Backspace on a
selected node opens it pre-cleared; arrow keys move the cursor
in grapheme units. Live edits paint through a `DeltaGlyphArea`
mutation against the tree (not the model) so the user sees
in-flight characters; on commit, the model is updated and a
single `EditNodeText` undo entry is pushed.

**Under the hood.** `src/application/app/text_edit/mod.rs:29-80`.
Cross-platform — works on both native and WASM. Cursor math
runs on grapheme-cluster indices throughout (via
[`grapheme_chad`](#utilities--grapheme_chad-color-geometry)),
so emoji and combining marks behave as single units. Original
text and regions are snapshotted on open; Esc restores them.

### Inline edge-label editor

**Summary.** Single-line text editing for line-mode edge labels.

**What it's for.** Setting or changing an edge label without
leaving the canvas. Same lifecycle as the node editor (commit
on click outside the AABB, cancel on Esc) but restricted to one
line.

**Under the hood.** `src/application/app/label_edit.rs`.
Native-only today. WASM users reach the same operation via the
`label` console verb, which has full cross-platform parity.

### Inline portal-text editor

**Summary.** Single-line text editing for portal-endpoint text.

**What it's for.** Setting the text label that sits next to a
portal icon. Selection and editing target one endpoint at a
time; the other endpoint's text is unaffected.

**Under the hood.** Mirrors the label editor shape; native-only
today, console parity on WASM.

### Glyph-wheel color picker

**Summary.** A modal HSV picker rendered as a 24-glyph hue ring
with sat/value crosshairs and theme-variable quick-pick chips.

**What it's for.** Picking a colour for the current selection
without leaving the canvas. Hover live-previews through the
`color_picker_preview` transient on `MindMapDocument`; the
scene builder reads it during render and substitutes the
preview colour for the targeted element. Click commits, click
outside cancels. Keyboard: h/H nudges hue, s/S sat, v/V value,
Tab cycles theme chips, Enter commits, Esc cancels.

**Under the hood.** `src/application/color_picker/mod.rs:1-77`
and `src/application/color_picker_overlay/`. Native-only today.
`compute_color_picker_layout()` is a pure function over
geometry + viewport, so layout can be unit-tested without GPU.
Two modes: contextual (modal, opened from edge context menu;
commits to the targeted edge and closes) and standalone
(persistent palette, opened via `color picker on`; commits to
the current selection and stays open).

### Console

**Summary.** A CLI-style command palette (Ctrl+;) for mutations,
styling, settings, and document operations.

**What it's for.** Power-user operations that don't have a
keybind. The console covers the long tail: zoom-bound
authoring, font-size clamps, palette swaps, mutation listing
and application, FPS toggle. Tokenised shell-style
(whitespace-split, `"quoted"` preserves spaces, `key=value`
first-class). Tab-completion is contextual and prefix-matched;
scrollback shows command history with dimmed older lines.

**Under the hood.** `src/application/console/mod.rs:1-170`.
Native-only today. Verbs include `zoom`, `font`, `color`,
`label`, `edge`, `portal`, `anchor`, `body`, `cap`, `spacing`,
`fps`, `mutation` (with `list`, `help`, `apply`, `inspect`
subverbs), `open`, `new`, `quit`, `save`. Visuals borrow
`baumhard::mindmap::border::BorderGlyphSet::box_drawing_rounded`
for the frame; content is clipped via
`grapheme_chad::truncate_to_display_width` so wide CJK
characters never overflow.

**Vision.** Console parity on WASM is the obvious next step;
the verb implementations are already cross-platform, only the
modal shell is native-gated.

### Keybinds and `Action`

**Summary.** A three-layer pipeline: abstract `Action` enum →
parsed `KeyBind` → resolved table; with cross-platform
configuration via XDG (native) and `?keybinds=` /
`localStorage` (WASM).

**What it's for.** Every keystroke that does *anything* maps to
an `Action` first; the `Action` is then dispatched in the right
input context (Document, Console, ColorPicker, LabelEdit,
TextEdit). This indirection means users can rebind keys without
touching code, and the same `Action` works on both targets even
though the config-loading paths differ.

**Under the hood.** `src/application/keybinds/`. The three
layers:

- `Action` enum (`action.rs`) — high-level intents:
  `Undo`, `CreateOrphanNode`, `EnterReparentMode`,
  `EnterConnectMode`, `DeleteSelection`, `EditSelection`,
  `OpenConsole`, `Copy`, `Paste`, `Cut`, `CancelMode`, etc.
- `KeyBind` parser (`bind.rs`) — string syntax like `"Ctrl+Z"`
  → modifier mask + key code.
- `ResolvedKeybinds` (`resolved.rs`) — fast `O(1)` lookup
  table built from `KeybindConfig` at startup.

`Action::context()` returns the input context the action
belongs to; the event loop uses it to filter eligible actions
based on which modal is open. Native config: hardcoded defaults
+ `$XDG_CONFIG_HOME/mandala/keybinds.json` + optional
`--keybinds <path>` CLI override. WASM: same defaults + query
param + `localStorage`. Partial configs merge via serde
`default` attributes.

### Clipboard

**Summary.** Cross-platform copy / cut / paste, with native
backed by `arboard` and WASM stubbed pending async-clipboard
integration.

**What it's for.** Selection-routed clipboard: each
[`SelectionState`](#selectionstate) variant has its own channel.
Copying a node copies its style and text; copying an edge
copies the body colour; copying an edge label copies the label
colour; copying a portal label copies the icon colour; copying
a portal text copies the text colour. The font channel mirrors
this routing for `font size= min= max=` writes.

**Under the hood.** `src/application/clipboard.rs:1-40`.
Failures (permission denied, unavailable) log via `log::warn!`
and return `None` — interactive paths must not panic. WASM
stubs warn-and-noop pending the browser's async clipboard API.

### `maptool` CLI

**Summary.** A separate binary in `crates/maptool/` for
scripted operations on `.mindmap.json` files: `show`, `grep`,
`apply`, `export`, `convert --legacy`, `convert --portals`,
`verify`.

**What it's for.** Authoring and maintenance from outside the
app. `verify` is the structural-invariant checker
([`format/validation.md`](./format/validation.md)). `convert`
migrates legacy formats. `apply` pipes node text through an
external command for batch edits. `export` renders to Markdown.
`grep` and `show` are read-only inspectors.

**Under the hood.** `crates/maptool/`. Not the focus of this
document — see the crate directly for the verb-level reference.
The format docs under [`format/`](./format/) are the
authoritative reference for what `verify` enforces.

---

## §7 Platform & parity

Mandala targets native desktop, browser on desktop, and browser
on mobile, equally. The lowest-spec target — a mid-range phone
in a browser — sets the performance budget. This section makes
the parity surface explicit, with each gap named so a reader
knows it is *not* "handled somewhere".

The authoritative live status is the **Dual-target status**
section of [`CLAUDE.md`](./CLAUDE.md). The list below is the
conceptual angle on the same surface.

### `PlatformContext`

**Summary.** A three-variant enum — `Desktop`, `Web`, `Touch` —
passed to mutation handlers and trigger-binding filters.

**What it's for.** Some operations should behave differently on
different targets — a layout that reflows narrower on mobile,
a trigger binding that only fires on Desktop. `PlatformContext`
is the channel for that distinction.

**Under the hood.** Defined in
`baumhard::mindmap::custom_mutation`. Today the variant is
chosen at compile time (Desktop on native, Web on WASM); the
`Touch` variant exists but no input path dispatches on it yet.

### Native-only surfaces

The following are native-only today, with reasons:

- **Drag gestures** — pan, move-node, edge-handle, portal-label,
  rect-select, edge-label. The whole `DragState` enum is
  native-gated; the cross-platform story will be touch-first
  recognisers feeding the same `ThrottledDrag` variants.
- **`AppMode::Reparent` / `AppMode::Connect`** — modal
  selection state and click routing; not yet wired on WASM.
- **Modals: console, glyph-wheel color picker, edge-label
  editor, portal-text editor.** Operations all have console
  verbs that work on both targets; only the modal shells are
  native-gated.
- **Hover-based UI** — `hovered_node` tracking, cursor change
  on button nodes, `OnClick` trigger preview.
- **Clipboard read/write** — `arboard` on native; WASM stubs.
- **`FreezeWatchdog`** — browsers provide their own
  unresponsive-tab handling.
- **FPS console verb** — the render-side plumbing builds on both
  targets; only the verb is native-gated because the console is.

### Absent on both targets

These are gaps, not "handled somewhere":

- **Touch gestures** — the `PlatformContext::Touch` variant
  exists but no input path dispatches on it.
- **DPI-aware canvas sizing on WASM** — the canvas tracks CSS
  pixels 1:1; no `devicePixelRatio` handling.
- **Runtime platform detection** — always the compile-time
  Desktop/Web branch.

### `cfg`-guard discipline

Native-only code sits behind
`#[cfg(not(target_arch = "wasm32"))]`; WASM-only behind
`#[cfg(target_arch = "wasm32")]`; everything else compiles for
both. Traits would have been the wrong abstraction here — a
trait implies an interface contract, but the platform split is
an availability split (clipboard does not exist in the same
sense on WASM, not "clipboard is implemented differently"). See
[`CODE_CONVENTIONS.md §4`](./CODE_CONVENTIONS.md). The
`./test.sh` WASM type-check gate and `./build.sh --wasm` enforce
the discipline locally.

---

## §8 Named trajectory — vision

The "named trajectory" is the project's set of explicit future
directions ([`CODE_CONVENTIONS.md §7`](./CODE_CONVENTIONS.md)).
Where the codebase is wider than today's use strictly demands,
it is because one of these is expected to attach there. None of
the following are speculation; each has a concrete seam and a
concrete consumer in mind. None are committed timelines either —
the project moves at the pace of one cathedral stone at a time.

Entries in this section are prose rather than layered concepts
— they are pointers into the rest of the document, not
definitions. Many of them mirror `Vision` layers on the
concepts they attach to; the duplication is intentional, local
context in the concept entry, cross-cutting picture here.

### Plugins and the Baumhard script API

The largest seam. Plugins will reach into the
[`CustomMutation`](#custommutation) registry, the
[`MutatorNode`](#declarative-path--mutatornode-ast) AST, the
[`NodeShape`](#nodeshape) variants, and the
[`SectionContext`](#runtime-holes--sectioncontext) trait. The
`"plugin.<name>.<kind>"` context namespace is reserved for
this. A Baumhard script API will compose the same primitives
from outside the Rust crate — declarative authoring of
mutators, animations, predicates — without recompiling the app.

### Richer animations

The reserved `Followup` slot on
[`AnimationTiming`](#animation-timing) is where chain, loop, and
reverse semantics will land. Per-field easing curves and spring
physics are the next layer. The
[`Timeline`](#animation-primitives--animationdef-animationinstance-timeline-timelineevent)
vocabulary is already mutator-based, so new motion primitives
attach as new `TimelineEvent` variants without rewriting the
driver.

### Complex file exports

The [scene builder](#scene-builder) is the export staging
ground: it already produces a flat element list with
glyph-positional truth. Complex exports — animated SVGs,
print-ready PDFs, frame-by-frame video — would consume the same
intermediate, with the renderer becoming one consumer among
many.

### Touch gestures

The [`PlatformContext::Touch`](#platformcontext) variant is the
route. Tap, pinch, and long-press recognisers will feed the
same [`ThrottledDrag`](#throttledinteraction-and-throttleddrag)
variants the mouse path uses today, so the mutation pipeline
does not change — only the input adapters.

### Shape-aware borders

Today's [borders](#border-geometry) only render around
rectangular nodes. Adding ellipse borders means per-shape glyph
layout — projecting box-drawing chars onto the curve — and the
[`NodeShape`](#nodeshape) seam is already the right place for
that branching.

### Palette transitions

Animated palette swaps will compose
[`Palettes`](#palettes) with
[animation timing](#animation-timing): a custom mutation that
interpolates `ColorGroup` fields between two palettes over a
duration, fired from a theme-switch trigger.

### Zoom-triggered LOD mutations

`GlyphAreaField::ZoomVisibility` already carries the mutator
target; what remains is a dispatcher that fires custom
mutations on zoom-window crossings. A node could swap its
content (summary at low zoom, full detail at high zoom)
declaratively, with the transition animated through the same
timing primitives.

### Runtime platform detection

Today the [`PlatformContext`](#platformcontext) is a
compile-time branch. Replacing it with a runtime value unblocks
correct behaviour on touch-capable desktop browsers (a Surface
laptop, an iPad in desktop mode). No primitive changes; just
move the enum from a `cfg` to a runtime check at app init.

### Reparent ID cascade

[Dewey IDs](#dewey-decimal-ids) currently drift on runtime
reparent — the old shape persists in the key while
`parent_id` becomes the truth. A future cascade pass will
renumber on reparent so the keys stay self-consistent, with
edges updated and undo coverage. `maptool verify` already
detects the drift; the rename pass will close the loop.

### Plugin / scripting parity for `Action` and trigger bindings

The [`Action`](#keybinds-and-action) enum is closed today;
opening it to plugin-registered actions (and matching
trigger-binding kinds) would let plugins surface their
operations through keybinds and `OnKey` bindings without core
changes. The dispatch is already context-filtered, so this is
mostly a registry change.

---

## §9 Glossary index

A flat, alphabetised quick-reference. Each entry points back to
its full treatment above.

- **Action** — keybind-resolvable user intent.
  See [§6: Keybinds](#keybinds-and-action).
- **animation timing** — duration / delay / easing wrapper on a
  custom mutation. See [§4: Animation timing](#animation-timing).
- **`AnimationDef` / `AnimationInstance`** — immutable animation
  blueprint and its per-playback state. See
  [§2: Animation primitives](#animation-primitives--animationdef-animationinstance-timeline-timelineevent).
- **`AnchorBox` / `Anchor`** — layout-pinning constraints stored
  on a `Flag::Anchored`. See [§2: Flag](#flag--flaggable--anchorbox).
- **`Applicable<T>`** — one-method dispatch trait for "apply this
  delta to that target". See [§2: `Applicable`](#applicablet).
- **`AppFont`** — compile-time enum of available fonts. See
  [§2: Font system](#font-system--font_system-appfont-attrs_list_from_regions).
- **`AppMode`** — modal state for reparent / connect operations.
  See [§5: `AppMode`](#appmode).
- **`AppScene`** — two-role (canvas / overlay) scene container.
  See [§5: `AppScene` and scene host](#appscene-and-scene-host).
- **`Application` / `InitState` / `NativeApp`** — native event-loop
  entry points. See [§5: Application, InitState, NativeApp](#application-initstate-nativeapp).
- **`ApplyOperation`** — Add / Assign / Subtract / Multiply /
  Delete / Noop selector. See [§2: `ApplyOperation`](#applyoperation).
- **`attrs_list_from_regions`** — bridge from `ColorFontRegions` to
  cosmic-text. See [§2: Font system](#font-system--font_system-appfont-attrs_list_from_regions).
- **Baumhard** — the glyph-animation library under
  `lib/baumhard/`. See [§1: Mandala and Baumhard are one project](#mandala-and-baumhard-are-one-project).
- **behavior** — `Persistent` or `Toggle` on a custom mutation.
  See [§4: Behaviors](#behaviors--persistent-vs-toggle).
- **border geometry** — glyph-drawn frames around mindmap nodes.
  See [§3: Border geometry](#border-geometry).
- **`border_t`** — parameter `[0, 4)` along a node's rectangular
  border, side-indexed clockwise from top-left. See
  [§3: Portal geometry](#portal-geometry).
- **`BranchChannel`** — the walker-alignment trait. See
  [§2: Channel and BranchChannel](#channel-and-branchchannel).
- **`Canvas`** — per-map rendering context (background, theme
  variables, defaults). See [§3: Canvas](#canvas).
- **`Camera2D` / `CameraMutation`** — pan/zoom projection and
  intent vocabulary. See [§2: Camera2D and CameraMutation](#camera2d-and-cameramutation).
- **canvas / overlay roles** — the two `AppScene` slot kinds.
  See [§5: AppScene and scene host](#appscene-and-scene-host).
- **`cfg`-guard discipline** — how platform-only code is gated.
  See [§7: cfg-guard discipline](#cfg-guard-discipline).
- **channel (Baumhard)** — integer routing tag on every node. See
  [§2: Channel](#channel-and-branchchannel).
- **channel (mindmap)** — the `MindNode.channel` field. See
  [§3: Channels (mindmap level)](#channels-mindmap-level).
- **clipboard** — selection-routed copy/cut/paste. See
  [§6: Clipboard](#clipboard).
- **`closest_point_on_path`** — projection of cursor onto a
  connection path. See [§3: Connection paths](#connection-paths).
- **`color_picker_preview`** — transient on `MindMapDocument` for
  hover live-preview. See [§6: Glyph-wheel color picker](#glyph-wheel-color-picker).
- **`ColorFontRegions`** — character-range span table. See
  [§2: ColorFontRegions](#colorfontregions).
- **`ColorSchema`** — a `MindNode`'s palette binding
  (`palette`, `level`, flags). See [§3: Palettes](#palettes).
- **`Comparator`** — comparison operator inside a `Predicate`.
  See [§2: Predicate and Comparator](#predicate-and-comparator).
- **connection paths** — straight or cubic Bézier between
  endpoints. See [§3: Connection paths](#connection-paths).
- **`ControlPoint`** — author-set Bézier offset from a node
  centre on a `MindEdge`. See [§3: ControlPoint](#controlpoint).
- **console** — native CLI command palette. See
  [§6: Console](#console).
- **contexts taxonomy** — `internal` / `map` / `map.node` /
  `map.tree` / `plugin.*` namespaces. See
  [§4: Contexts taxonomy](#contexts-taxonomy).
- **`CustomMutation`** — named, reusable operation carrier. See
  [§4: CustomMutation](#custommutation).
- **declarative path** — `MutatorNode` AST → `MutatorTree` →
  walker. See [§4: Declarative path — MutatorNode AST](#declarative-path--mutatornode-ast).
- **`DeltaGlyphArea` / `DeltaGlyphModel`** — batched-operation
  deltas on `GlyphArea` / `GlyphModel`. See
  [§2: GlyphAreaField and DeltaGlyphArea](#glyphareafield-and-deltaglypharea).
- **Dewey-decimal IDs** — dot-separated hierarchical node IDs.
  See [§3: Dewey-decimal IDs](#dewey-decimal-ids).
- **dirty flag** — single `bool` on `MindMapDocument` gating
  scene rebuild. See [§5: Dirty flag](#dirty-flag).
- **document actions** — canvas-level operations on a custom
  mutation. See [§4: Document actions](#document-actions).
- **`drain_frame`** — per-frame heartbeat. See
  [§5: Event loop and drain_frame](#event-loop-and-drain_frame).
- **`DragState`** — mouse-drag state machine. See
  [§5: DragState](#dragstate).
- **`DynamicMutationHandler`** — registered Rust function pointer
  for imperative custom mutations. See
  [§4: Imperative path — DynamicMutationHandler](#imperative-path--dynamicmutationhandler).
- **edge label** — `EdgeLabelConfig` + positioned text on a
  line-mode edge. See [§3: Edge labels](#edge-labels).
- **`EdgeRef`** — `(from_id, to_id, edge_type)` identity tuple.
  See [§5: EdgeRef](#edgeref).
- **`EdgeLabel` selection** — `SelectionState::EdgeLabel` variant.
  See [§5: SelectionState](#selectionstate).
- **`EventSubscriber`** — `Arc<Mutex<dyn FnMut...>>` callback for
  reactive mutators. See
  [§2: Event, GlyphTreeEvent, GlyphTreeEventInstance, EventSubscriber](#event-glyphtreeevent-glyphtreeeventinstance-eventsubscriber).
- **everything is glyphs** — all visuals are positioned font
  glyphs. See [§1: Everything is glyphs](#everything-is-glyphs).
- **`Flag` / `Flaggable`** — per-node state markers. See
  [§2: Flag, Flaggable, AnchorBox](#flag--flaggable--anchorbox).
- **fold state** — `MindNode.folded` boolean. See
  [§3: Fold state](#fold-state).
- **`FONT_SYSTEM`** — global cosmic-text `FontSystem`. See
  [§2: Font system](#font-system--font_system-appfont-attrs_list_from_regions).
- **four-source loader** — App / User / Map / Inline mutation
  precedence. See [§4: Four-source loader](#four-source-loader).
- **FPS overlay** — snapshot or debug frame-rate readout. See
  [§5: FPS overlay](#fps-overlay).
- **frame throttle** — adaptive mutation-frequency throttle. See
  [§5: MutationFrequencyThrottle](#mutationfrequencythrottle-and-frame_throttle).
- **`FreezeWatchdog`** — native background thread aborting on
  main-loop hang. See [§5: FreezeWatchdog](#freezewatchdog).
- **`GfxElement`** — tree-node enum (`GlyphArea`, `GlyphModel`,
  `Void`). See [§2: GfxElement](#gfxelement).
- **`GfxMutator`** — mutator-side node enum. See
  [§2: GfxMutator](#gfxmutator).
- **glossary index** — this section.
- **`GlyphArea`** — text region element. See
  [§2: GlyphArea](#glypharea).
- **`GlyphAreaCommand`** — named-operation mutations on
  `GlyphArea`. See [§2: GlyphAreaCommand](#glyphareacommand).
- **`GlyphAreaField`** — per-field delta surface for
  `GlyphArea`. See [§2: GlyphAreaField and DeltaGlyphArea](#glyphareafield-and-deltaglypharea).
- **`GlyphBorderConfig`** — per-node border style record. See
  [§3: Border geometry](#border-geometry).
- **`GlyphConnectionConfig`** — per-edge glyph rendering config
  (body, caps, font, size clamps, colour). See
  [§3: GlyphConnectionConfig](#glyphconnectionconfig).
- **glyph-wheel color picker** — modal HSV picker. See
  [§6: Glyph-wheel color picker](#glyph-wheel-color-picker).
- **`GlyphComponent`** — text + font + colour triplet, leaf of
  the model hierarchy. See
  [§2: GlyphModel, GlyphMatrix, GlyphLine, GlyphComponent](#glyphmodel-glyphmatrix-glyphline-glyphcomponent).
- **`GlyphLine`** — one line of components. See same.
- **`GlyphMatrix`** — vertical stack of lines. See same.
- **`GlyphModel`** — composed-glyph shape, child of a `GlyphArea`.
  See same.
- **`GlyphModelField` / `DeltaGlyphModel` / `GlyphModelCommand`**
  — the mutation surface for `GlyphModel`, parallel to the area
  trio. See [§2: GlyphModelField, DeltaGlyphModel, GlyphModelCommand](#glyphmodelfield-deltaglyphmodel-glyphmodelcommand).
- **`GlyphTreeEvent` / `GlyphTreeEventInstance`** — event kinds
  and timestamped instances. See
  [§2: Event, GlyphTreeEvent, ...](#event-glyphtreeevent-glyphtreeeventinstance-eventsubscriber).
- **grapheme_chad** — Unicode-correct text primitives. See
  [§2: Utilities](#utilities--grapheme_chad-color-geometry).
- **imperative path** — `DynamicMutationHandler` registry. See
  [§4: Imperative path](#imperative-path--dynamicmutationhandler).
- **`Instruction`** — `RepeatWhile` / `SpatialDescend` /
  `MapChildren` / `RotateWhile` (reserved). See
  [§2: Instruction](#instruction).
- **inline editors** — node-text, edge-label, portal-text. See
  [§6: Inline node-text editor](#inline-node-text-editor),
  [§6: Inline edge-label editor](#inline-edge-label-editor),
  [§6: Inline portal-text editor](#inline-portal-text-editor).
- **inline mutations** — `MindNode.inline_mutations`, the
  highest-precedence layer of the four-source loader. See
  [§4: Four-source loader](#four-source-loader).
- **keybinds** — three-layer Action / KeyBind / ResolvedKeybinds
  pipeline. See [§6: Keybinds and Action](#keybinds-and-action).
- **`MapChildren`** — instruction that pairs mutator children to
  target children by sibling position. See
  [§2: Instruction](#instruction).
- **Mandala** — the mindmap application. See
  [§1: Mandala and Baumhard are one project](#mandala-and-baumhard-are-one-project).
- **`maptool`** — CLI for `.mindmap.json` operations. See
  [§6: maptool CLI](#maptool-cli).
- **`MindEdge`** — directed connection between two nodes. See
  [§3: MindEdge](#mindedge).
- **`MindMap`** — document root struct. See
  [§3: MindMap](#mindmap).
- **`MindMapDocument`** — application-layer document owner. See
  [§5: MindMapDocument](#mindmapdocument).
- **`MindNode`** — one mindmap node. See
  [§3: MindNode](#mindnode).
- **model / view separation** — document owns data, renderer owns
  GPU resources. See
  [§1: Model / view separation](#model--view-separation).
- **`Mutation` enum** — payload union for `GfxMutator`. See
  [§2: Mutation enum](#mutation-enum).
- **mutation-first** — every change is a mutator applied to a
  tree. See [§1: Mutation-first](#mutation-first).
- **`MutationSource`** — provenance tag (App / User / Map /
  Inline) on each merged custom mutation. See
  [§4: Four-source loader](#four-source-loader).
- **`MutationFrequencyThrottle`** — adaptive frame throttle. See
  [§5: MutationFrequencyThrottle](#mutationfrequencythrottle-and-frame_throttle).
- **mutator builder DSL** — `MutatorNode` AST and
  `SectionContext`. See
  [§2: Mutator builder DSL](#mutator-builder-dsl--mutatornode-sectioncontext-repeat-runtime-holes).
- **`MutatorNode`** — serde-friendly mutator AST. See same and
  [§4: Declarative path](#declarative-path--mutatornode-ast).
- **`MutatorTree<M>`** — mutation-side tree mirror. See
  [§2: MutatorTree](#mutatortreem).
- **named trajectory** — explicit future directions. See
  [§8: Named trajectory — vision](#8-named-trajectory--vision).
- **`NodeShape`** — pluggable hit-test shape. See
  [§2: NodeShape](#nodeshape).
- **`now_ms`** — cross-platform monotonic clock. See
  [§5: now_ms](#now_ms).
- **`OutlineStyle`** — eight-stamp glyph halo. See
  [§2: OutlineStyle](#outlinestyle).
- **palettes** — map-level named colour schemes. See
  [§3: Palettes](#palettes).
- **`PlatformContext`** — Desktop / Web / Touch enum. See
  [§7: PlatformContext](#platformcontext).
- **portal geometry** — `border_t` ↔ canvas point conversion.
  See [§3: Portal geometry](#portal-geometry).
- **`PortalLabel` selection** — `SelectionState::PortalLabel`
  (the icon channel). See [§5: SelectionState](#selectionstate).
- **`PortalText` selection** — `SelectionState::PortalText` (the
  text channel). See same.
- **portals** — edges with `display_mode = "portal"`. See
  [§3: Portals](#portals).
- **`Predicate`** — element-matching condition. See
  [§2: Predicate and Comparator](#predicate-and-comparator).
- **preserved seam** — extension point preserved at full width
  even when narrowly used today. See
  [§1: Preserved seams](#preserved-seams).
- **`Range`** — half-open `[start, end)` span. See
  [§2: Range](#range).
- **`RegionIndexer` / `RegionParams` / `RegionError`** — spatial
  index over `ColorFontRegions`. See
  [§2: RegionParams, RegionIndexer, RegionError](#regionparams-regionindexer-regionerror).
- **`Renderer`** — GPU resource holder. See
  [§5: Renderer](#renderer).
- **`RenderScene` elements** — flat element structs
  (`TextElement`, `BorderElement`, `ConnectionElement`,
  `PortalElement`, `ConnectionLabelElement`, `EdgeHandleElement`)
  forming the scene builder's output. See
  [§3: Scene builder](#scene-builder).
- **`RepeatWhile`** — predicate-gated loop instruction. See
  [§2: Instruction](#instruction).
- **runtime hole** — `Runtime("<label>")` value resolved by
  `SectionContext`. See [§4: Runtime holes](#runtime-holes--sectioncontext).
- **scene builder** — `MindMap` → `RenderScene`. See
  [§3: Scene builder](#scene-builder).
- **scene cache** — per-edge sampled-position cache. See
  [§3: Scene cache](#scene-cache).
- **scene rebuild granularity** — five tiered rebuild functions.
  See [§5: Scene rebuild granularity](#scene-rebuild-granularity).
- **`Scene`** — multi-tree compositor. See
  [§2: Scene](#scene).
- **`SectionContext`** — runtime-value provider for the mutator
  builder. See [§4: Runtime holes](#runtime-holes--sectioncontext).
- **`SelectionState`** — what the user has selected. See
  [§5: SelectionState](#selectionstate).
- **single-threaded event loop** — no worker threads in
  interactive paths. See
  [§1: Single-threaded event loop](#single-threaded-event-loop).
- **`SpatialDescend`** — instruction that descends to the
  deepest node containing a point. See
  [§2: Instruction](#instruction).
- **target scope** — node window covered by a custom mutation.
  See [§4: Target scopes](#target-scopes).
- **text runs** — character-range rich-text styling on a
  `MindNode`. See [§3: Text runs](#text-runs).
- **theme variables** — document-level named colours referenced
  as `var(--name)`. See [§3: Theme variables](#theme-variables).
- **`ThrottledDrag` / `ThrottledInteraction`** — adaptive-throttle
  drag enum and its trait. See
  [§5: ThrottledInteraction and ThrottledDrag](#throttledinteraction-and-throttleddrag).
- **`Timeline` / `TimelineEvent`** — animation event sequence.
  See [§2: Animation primitives](#animation-primitives--animationdef-animationinstance-timeline-timelineevent).
- **toggle behavior** — `Behavior::Toggle` on a custom mutation;
  reverses on second trigger. See [§4: Behaviors](#behaviors--persistent-vs-toggle).
- **tree builder** — `MindMap` → `Tree<GfxElement, GfxMutator>`.
  See [§3: Tree builder](#tree-builder).
- **`Tree<T, M>`** — arena-backed glyph forest. See
  [§2: Tree](#treet-m).
- **`TreeWalker`** — mutation dispatch engine
  (`walk_tree_from`). See [§2: TreeWalker](#treewalker).
- **trigger bindings** — per-node input → custom-mutation
  dispatch. See [§3: Trigger bindings](#trigger-bindings).
- **`UndoAction`** — 12-variant undo enum. See
  [§5: UndoAction](#undoaction).
- **`Void`** — no-op tree node. See [§2: Void](#void).
- **`ZoomVisibility`** — presence-gating zoom window. See
  [§2: ZoomVisibility](#zoomvisibility) and
  [§3: Zoom bounds](#zoom-bounds).

---

*This document is a living foundation. As Mandala and Baumhard
grow, entries will be added, sharpened, sometimes deleted (per
[`CODE_CONVENTIONS.md §10`](./CODE_CONVENTIONS.md), delete
rather than deprecate). When a concept stops being load-bearing,
its entry comes out — keeping the reference honest is part of
keeping the project honest.*

