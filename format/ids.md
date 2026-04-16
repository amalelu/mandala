# Structural IDs

Node IDs in `.mindmap.json` are **Dewey-decimal paths** that encode the
node's position in the tree.

```
"0"       — first root
"1"       — second root
"1.0"     — first child of root 1
"1.0.2"   — third grandchild, under root 1's first child
"2.3.1.0" — deeper still
```

The ID is the tree path; reading a file tells you the shape of the tree
without tracing `parent_id` pointers.

## Why not UUIDs or heap pointers?

The legacy miMind format used 32-bit heap pointers converted to strings
(`"348068464"`). They were opaque: you couldn't read a file and reason
about the hierarchy. UUIDs would be worse.

Hand-authored maps in the early development period used symbolic names
(`"title"`, `"child_a"`) — readable, but only while the author remembered
what each name meant.

Dewey-decimal IDs are self-documenting. A reviewer reading the JSON sees
the structure in the keys:

```
"0"      "Lord God"
"0.0"    "commanded the man"
"0.0.0"  "You are free to eat..."
"1"      "Adam & Eve"
"1.0"    "Cain"
"1.0.0"  "Enoch"
```

## The relationship to `parent_id`

Every node still carries an explicit `parent_id` field. The ID encodes
structure; `parent_id` caches it for O(1) parent lookup without string
parsing.

The two must agree. For any non-root node:
- The Dewey-derived parent (everything before the last dot) must equal
  `parent_id`
- `parent_id` is null iff the ID has no dot (root node)

`maptool verify` checks this invariant.

## Sibling order

Siblings sort by the **last segment** of their ID, parsed as an integer:

```
"1.0" < "1.1" < "1.2" < "1.10"
```

The parse is numeric, not lexicographic — `"1.10"` comes after `"1.9"`,
not between `"1.1"` and `"1.2"`. This matches the Dewey intuition.

`id_sort_key` in `lib/baumhard/src/mindmap/model/mod.rs` implements this.

## ID stability across reparent

Currently, IDs **do not cascade** when a node is reparented via the UI.
If node `"1.2"` is moved under `"0"`, its ID stays `"1.2"` even though
its new parent is `"0"`. `parent_id` tracks the truth; the Dewey structure
degrades.

This is a known limitation and a tracked trajectory item. The mutation-time
cascade (rename the subtree, rewrite all references in edges/portals/undo)
is expensive and intrusive; we accept the drift in exchange for keeping
reparent cheap and undo simple.

**When IDs *do* cascade**: on `delete_node`, when children are orphaned
(promoted to root), the subtree's IDs are rewritten to reflect their new
position. Edges and portals are updated. Undo reverses the rename.

## ID assignment for new nodes

`fresh_child_id(parent: Option<&str>)` (in
`src/application/document/topology.rs`) returns the next available Dewey
segment under the given parent. If children `"1.0"`, `"1.1"`, `"1.2"` exist,
the next is `"1.3"`. If `"1.0"` and `"1.2"` exist but not `"1.1"`, the
next is still `"1.3"` — gaps are not reused. This keeps new IDs stable
across session history.

## Why not store the tree recursively?

Rust's ownership model makes recursive structures awkward. A flat
`HashMap<String, MindNode>` with `parent_id` references gives us:

- O(1) lookup by ID
- No lifetime juggling for parent/child references
- Simple serde serialization
- Easy partial updates during editing

Dewey IDs give us back the "you can see the tree in the data" property
that recursive structures have, without the ownership headache.
