# Mandala Mindmap - Architecture & Roadmap

## Context

Mandala is a Rust mindmap application built on WebGPU (wgpu) with the Baumhard
glyph animation library. It renders `.mindmap.json` files as interactive canvases
where every visual element — text, borders, connections — is a positioned font glyph.

The application has two layers:
- **MindMap data model** — flat `HashMap<String, MindNode>` with `parent_id` references forming a tree. Loaded from JSON, queried for hierarchy, fold state, and theme colors.
- **Baumhard mutation tree** — `Tree<GfxElement, GfxMutator>` built from the MindMap hierarchy. Each MindNode becomes a GlyphArea in the tree. MutatorTrees can be applied to cascade transformations (position, scale, color, text) through the parent-child structure. This is the creative engine — it enables animation and interactive manipulation of the mindmap.

**Key drivers:**

1. The Baumhard mutation tree is the core creative tool — selection highlights, node movement, fold animation, and visual effects should all be expressed as mutations cascading through the tree
2. The single-parent tree structure (`parent_id: Option<String>`) is preserved — it maps directly to Baumhard's indextree. Non-hierarchical relationships use arbitrary connections (`edge_type: "cross_link"`), not multi-parent
3. Editing capabilities needed — selection, move, reparent, connect, text editing
4. Connections and borders currently render via a flat pipeline alongside the tree; borders should eventually move into the tree as GlyphModel children

---

## Architectural Decisions

1. **Single-threaded architecture** — Direct method calls, no message channels. Matches WASM reality, simpler state management
2. **Model-View separation** — MindMapDocument owns the MindMap, provides `build_tree()` and `build_scene()`
3. **Dual rendering pipeline** — Nodes render through the Baumhard tree (tree_builder → Tree → renderer walks tree for cosmic-text buffers). Connections and borders render through the flat RenderScene pipeline (scene_builder → flat Vecs → renderer). Borders will eventually migrate into the tree as GlyphModel children
4. **Single-parent tree** — `parent_id: Option<String>` maps to indextree. Non-hierarchical links are arbitrary connections, not multi-parent nodes
5. **Everything is glyphs** — Text, borders, connections all rendered as positioned font glyphs via cosmic-text/glyphon
6. **Mutations as the interaction model** — User actions (select, move, edit) should be expressed as MutatorTree applications where possible, so the tree walker handles cascading effects naturally

---

## Current State (after Session 4)

### What works
- **MindMap data model** — serde-based structs, JSON loading, hierarchy queries
- **Baumhard tree bridge** — MindMap → Tree<GfxElement, GfxMutator> with parent-child hierarchy preserved
- **Node rendering via tree** — renderer walks Baumhard tree to create cosmic-text buffers
- **Border rendering** — glyph-based box-drawing borders (flat pipeline)
- **Connection rendering** — glyph-based edge paths with Bezier curves (flat pipeline)
- **Camera** — pan/zoom with fit-to-content
- **Node selection** — click to select, shift+click for multi-select, click empty to deselect
- **Selection highlight** — selected nodes highlighted via GlyphArea color region mutation (cyan)
- **Click vs drag** — left-click selects, left-drag pans (5px threshold), middle-drag pans
- **Multi-target** — native + WASM builds
- **126 tests passing**

### What needs work
- **No node movement** — selection exists but nodes can't be dragged to new positions
- **No text editing** — no inline text editing or node creation
- **Borders not in tree** — borders render via flat pipeline, not as GlyphModel children in the Baumhard tree
- **No save/persistence** — document dirty flag exists but no serialization
- **WASM input** — selection not yet wired up on WASM (TODOs in event loop)

### Key Files Reference
| File | Role |
|------|------|
| `src/application/app.rs` | Event loop, window, tree+scene pipeline wiring |
| `src/application/renderer.rs` | GPU pipeline: tree-based node rendering + flat border/connection rendering |
| `src/application/document.rs` | Owns MindMap + SelectionState, provides `build_tree()`, `build_scene()`, `hit_test()`, `apply_selection_highlight()` |
| `src/application/common.rs` | RenderDecree, WindowMode, InputMode, timing |
| `lib/baumhard/src/mindmap/model.rs` | MindMap, MindNode, MindEdge structs |
| `lib/baumhard/src/mindmap/tree_builder.rs` | MindMap → Tree<GfxElement, GfxMutator> bridge |
| `lib/baumhard/src/mindmap/scene_builder.rs` | MindMap → flat RenderScene (connections, borders) |
| `lib/baumhard/src/mindmap/loader.rs` | JSON loading |
| `lib/baumhard/src/mindmap/border.rs` | BorderGlyphSet, BorderStyle |
| `lib/baumhard/src/mindmap/connection.rs` | Path computation, Bezier curves, glyph sampling |
| `lib/baumhard/src/gfx_structs/tree.rs` | Tree<T,M>, MutatorTree<T> |
| `lib/baumhard/src/gfx_structs/tree_walker.rs` | Mutation tree walking (channel-aligned, with RepeatWhile) |
| `lib/baumhard/src/gfx_structs/element.rs` | GfxElement enum (GlyphArea, GlyphModel, Void) |
| `lib/baumhard/src/gfx_structs/mutator.rs` | GfxMutator, Mutation, Instruction, Predicate types |

---

## Milestone Dependency Graph

```
M1 (Architecture) ✓ --+--> M2 (Connections) ✓ ------+
                       |                              +--> M6 (Connection Editing)
                       +--> M3 (Tree Bridge) ✓ --+
                       |                          |
                       |    M3 enables mutation-  |
                       |    based interaction:    |
                       |                          +--> M4 (Selection) ✓
                       |                          |      via color/highlight mutations
                       |                          |
                       |                          +--> M5 (Move/Reparent)
                       |                          |      via position mutations cascading through tree
                       |                          |
                       |                          +--> M7 (Text Edit)
                       |                                 via GlyphArea text mutations
                       |
                       +--> M8 (Save/File) [can start any time]
```

---

## Milestone 1: Architecture Foundation

**Goal**: Clean separation of Model, Application Logic, and Rendering. Generic file loading.

### Session 1A: Remove game engine leftovers & simplify threading

**What**: Strip all game-specific code and collapse multi-threaded architecture.

- [x] Delete `src/application/game_concepts.rs` entirely (World, Scene, GameObject, etc.)
- [x] Delete `src/application/main_menu.rs`
- [x] Remove from `common.rs`: all game types (GameResourceType, GameStatModifier, GameObjectFlags, GameItemProperty, GameResourceAspect, StatModifier, etc. - lines 284-405)
- [x] Remove `Decree`/`Instruction`/`AckKey` channel infrastructure from `common.rs`
- [x] Remove `HostDecree` game variants (MasterSound*, Load/Save/Pause/ExitInstance)
- [x] Keep from `common.rs`: `WindowMode`, `InputMode`, `RedrawMode`, `KeyPress`, `StopWatch`, `PollTimer`
- [x] Refactor `app.rs` to single-threaded: remove crossbeam channels, collapse event loop to own Renderer directly
- [x] Ensure it compiles and renders the existing mindmap

**Verify**: `cargo build` succeeds, `cargo run` renders testament mindmap as before

### Session 1B: Introduce MindMapDocument and RenderScene

**What**: Clean Model-View separation with the Document and RenderScene abstractions.

- [x] Create `src/application/document.rs` with `MindMapDocument` struct (owns `MindMap`, `SelectionState`, `dirty` flag, `undo_stack`)
- [x] Move `self.mindmap` out of Renderer into Document
- [x] Create `lib/baumhard/src/mindmap/scene_builder.rs` with `RenderScene` struct
- [x] Extract scene-building logic from `renderer.rs:582-716` (`rebuild_mindmap_buffers`) into scene_builder
- [x] `RenderScene` contains: `text_elements`, `border_elements` (connection/portal elements as empty Vecs for now)
- [x] Renderer receives `&RenderScene` and builds cosmic-text buffers from it
- [x] Application owns both Document and Renderer, wires them together

**Verify**: `cargo build` + `cargo run` renders same result, `cargo test` passes

### Session 1C: Generic file loading

**What**: Load any `.mindmap.json` from CLI args or WASM URL.

- [x] Accept mindmap file path as CLI argument (`std::env::args`)
- [x] Default to `maps/testament.mindmap.json` if no arg provided
- [x] For WASM: read from URL query parameter or embedded default
- [x] Application creates Document from loaded MindMap, passes to Renderer
- [x] Test with multiple mindmap files

**Verify**: `cargo run -- maps/testament.mindmap.json` works, `cargo run -- other.mindmap.json` works

---

## Milestone 1.5: Border Rendering

**Goal**: Render node borders using glyph-based box-drawing characters. Everything is glyphs.

**Status**: Complete (implemented pre-roadmap, documented here for tracking).

### Border system implementation

**What**: Node borders composed entirely of positioned font glyphs (Unicode box-drawing characters).

- [x] Create `lib/baumhard/src/mindmap/border.rs` with `BorderGlyphSet` and `BorderStyle`
- [x] Implement 4 glyph presets: light (`┌─┐`), heavy (`┏━┓`), double (`╔═╗`), rounded (`╭─╮`)
- [x] Generate top/bottom border strings with corner glyphs and repeated edge glyphs
- [x] Generate left/right side columns with repeated vertical glyphs
- [x] Add `GlyphBorderConfig` to JSON format with preset selection and custom glyph support
- [x] Add `CustomBorderGlyphs` for user-defined glyph overrides when preset = "custom"
- [x] Support per-node border config via `NodeStyle.border` field
- [x] Support canvas-level default border via `Canvas.default_border` field
- [x] Scene builder generates `BorderElement` for nodes with `show_frame = true`
- [x] Renderer builds 4 cosmic-text buffers per border (top, bottom, left, right segments)
- [x] Border color inherited from `frame_color`, overridable via config

**Key files**: `lib/baumhard/src/mindmap/border.rs`, `lib/baumhard/src/mindmap/model.rs` (GlyphBorderConfig), `lib/baumhard/src/mindmap/scene_builder.rs` (BorderElement), `src/application/renderer.rs` (border buffer rendering)

---

## Milestone 2: Connection Rendering

**Goal**: Render edges between nodes using glyph-based connections.

### Session 2A: Connection path computation

**What**: Compute paths between connected nodes and lay out glyphs along them.

- [x] Create `lib/baumhard/src/mindmap/connection.rs`
- [x] Implement path computation: source anchor -> target anchor (straight line)
- [x] Add Bezier curve support using existing `control_points` data
- [x] Sample points along path at intervals matching glyph spacing
- [x] Define anchor point system: 0=auto, 1-4=top/bottom/left/right

**Verify**: Unit tests for path computation and point sampling

### Session 2B: Glyph connection rendering

**What**: Generate render elements for connections and display them.

- [x] Add `ConnectionElement` to `RenderScene`
- [x] Scene builder generates connection elements from `MindEdge` data
- [x] Use `GlyphConnectionConfig.body` as repeating glyph (default: middle dot)
- [x] Place `cap_start`/`cap_end` glyphs at endpoints
- [x] Renderer builds cosmic-text buffers for connections
- [x] Fall back to canvas `default_connection` when edge has no `glyph_connection`

**Verify**: Edges visible between nodes in testament mindmap

---

## Milestone 3: Baumhard Tree Bridge

**Goal**: Bridge MindMap hierarchy into Baumhard's mutation tree system, enabling creative animation of the mindmap through the Tree<GfxElement, GfxMutator> + MutatorTree pipeline.

**Architectural decision**: Multi-parent support is dropped. The single-parent tree (`parent_id: Option<String>`) maps directly to Baumhard's indextree hierarchy and is the correct model. Non-hierarchical relationships use arbitrary connections (`edge_type: "cross_link"`), which don't participate in tree structure. Portal system is deferred to a later milestone.

### Session 3: MindMap-to-Baumhard tree pipeline

**What**: Build a Tree<GfxElement, GfxMutator> from the MindMap's parent-child hierarchy and render through it.

- [x] Create `lib/baumhard/src/mindmap/tree_builder.rs` with `build_mindmap_tree()`
- [x] Convert MindNode → GlyphArea (text, position, size, ColorFontRegions)
- [x] Recursively build tree following `parent_id` hierarchy
- [x] Exclude nodes hidden by fold state via `is_hidden_by_fold()`
- [x] Return `MindMapTree` with tree + node_map (MindNode ID → NodeId)
- [x] Add `rebuild_buffers_from_tree()` to Renderer (walks tree, creates cosmic-text buffers)
- [x] Add `fit_camera_to_tree()` to Renderer
- [x] Split out `rebuild_border_buffers()` and `rebuild_connection_buffers()` as flat pipeline
- [x] Add `build_tree()` to MindMapDocument
- [x] Wire up app.rs: nodes via tree, connections+borders via flat scene
- [x] 7 unit tests for tree structure, GlyphArea properties, hierarchy preservation
- [x] All 120 tests pass, visual rendering preserved

**Key files**:
- `lib/baumhard/src/mindmap/tree_builder.rs` (new) - MindMap → Baumhard Tree bridge
- `src/application/renderer.rs` - tree-based rendering path + split flat methods
- `src/application/document.rs` - `build_tree()` method
- `src/application/app.rs` - wired to tree flow (native + WASM)

**What this enables**: MutatorTree<GfxMutator> can now be applied to the mindmap tree, cascading mutations (position, scale, color, text) through the parent-child hierarchy. This is the foundation for creative animation of the mindmap.

**Verify**: `cargo test -p baumhard -p mandala` passes (120 tests at end of session 3), `cargo run -- maps/testament.mindmap.json` renders correctly

---

## Milestone 4: Selection & Basic Interaction

**Goal**: Users can select, highlight, and inspect nodes. Selection highlight expressed as a mutation on the Baumhard tree.

### Session 4A: Hit testing and selection

**What**: Click nodes to select them, with visual feedback via tree mutations.

- [x] Implement node hit testing: click position → Camera2D.screen_to_canvas() → find node by bounds using node_map + GlyphArea positions
- [x] Add `SelectionState` to Document (None, Single(String), Multi(Vec<String>))
- [x] Handle click events in app event loop → delegate to Document
- [x] Shift+click for multi-select, click empty space to deselect
- [x] Selection highlight via GlyphArea color region mutation (cyan highlight, restored on deselect via tree rebuild)
- [x] Rebuild tree and renderer buffers on selection change
- [x] Click vs drag distinction: left-click selects, left-drag pans (5px threshold)
- [x] 6 new tests: hit_test_direct_hit, hit_test_miss, hit_test_returns_smallest_on_overlap, selection_state_is_selected, apply_selection_highlight, highlight_does_not_affect_unselected

**Key files**:
- `src/application/document.rs` — SelectionState, hit_test(), apply_selection_highlight()
- `src/application/app.rs` — persistent document/tree in event loop, click handling, modifier tracking
- `src/application/renderer.rs` — screen_to_canvas()

**Verify**: `cargo test -p baumhard -p mandala` passes (126 tests), clicking nodes selects them with visual feedback

---

## Milestone 5: Node Editing - Move & Reparent

**Goal**: Drag nodes to reposition, reparent via drag-and-drop. Movement expressed as position mutations on the Baumhard tree.

### Session 5A: Node movement

**What**: Drag to move nodes, with subtree and individual modes.

- [ ] Implement drag gesture detection (mouse down → move → mouse up)
- [ ] Default drag: move node + all descendants — apply NudgeDown/NudgeRight MutatorTree to subtree, then sync positions back to MindMap model
- [ ] Alt+drag: move individual node only (apply mutation to single node, not descendants)
- [ ] Update positions in MindMap model, mark document dirty
- [ ] Rebuild tree from updated model (or apply mutations directly to existing tree)
- [ ] Add undo support: push `MoveAction` to undo stack

**Verify**: Nodes can be dragged around, subtrees move together

### Session 5B: Reparent via drag

**What**: Drag a node onto another to reparent it.

- [ ] Detect drag-over-node: highlight potential parent target
- [ ] On release over node: reparent (update `parent_id`, recalculate index)
- [ ] Support drop as first child, last child, or between siblings
- [ ] Rebuild Baumhard tree after reparent (tree structure changed)
- [ ] Visual indicators for drop position
- [ ] Undo support for reparent operations

**Verify**: Nodes can be reparented by dragging, undo restores previous state

---

## Milestone 6: Connection Editing

**Goal**: Create, delete, and fully customize connections between nodes.

### Session 6A: Connection selection and deletion

**What**: Select connections and delete them, with visual feedback.

- [ ] Click on a connection glyph path to select the edge
- [ ] Hit testing for connections: click position -> find nearest edge by glyph proximity
- [ ] Visual feedback: highlight selected connection (color change, thicker glyphs, or glow)
- [ ] Display edge info on selection (type, color, label, anchor points)
- [ ] Delete selected connection (Delete key or context action)
- [ ] Undo support for connection deletion (push `DeleteEdgeAction` to undo stack)

**Verify**: Connections can be clicked to select and deleted with undo

### Session 6B: Create connections

**What**: Draw new connections between nodes.

- [ ] "Connect mode": select source node, click target to create edge
- [ ] Visual feedback: temporary connection following cursor from source anchor
- [ ] Create new `MindEdge` with default glyph style from canvas config
- [ ] Support edge types: `parent_child`, `cross_link`
- [ ] Cross-links are arbitrary connections — they don't affect the tree hierarchy
- [ ] Undo support for connection creation

**Verify**: New connections can be created between any two nodes

### Session 6C: Connection path manipulation

**What**: Edit connection paths, control points, and anchor positions.

- [ ] Drag control points to curve existing straight connections (adds Bezier control points)
- [ ] Drag existing control points to reshape curved connections
- [ ] Visual handles: render draggable control point markers on selected connections
- [ ] Change anchor points: drag connection endpoints to different sides of a node (top/right/bottom/left)
- [ ] Snap anchor to nearest edge midpoint on release
- [ ] Reset connection to straight line (remove control points)
- [ ] Undo support for all path modifications

**Verify**: Connections can be curved, reshaped, and anchor points moved

### Session 6D: Connection style and label editing

**What**: Customize connection appearance and add/edit labels.

- [ ] Change connection glyph: body, cap_start, cap_end via selection panel or keyboard shortcut
- [ ] Change connection color (color picker or preset palette)
- [ ] Change connection font and font size
- [ ] Change glyph spacing (tight, normal, wide)
- [ ] Edit connection label: click to add/edit text label on connection
- [ ] Position label along connection path (start, middle, end, or custom offset)
- [ ] Change edge type (parent_child, cross_link, arbitrary) on existing connections
- [ ] Apply canvas default style to selected connections (reset to default)

**Verify**: Connection appearance can be fully customized, labels can be added and edited

### Session 6E: Portal creation and management

**What**: Create and manage portal pairs for non-hierarchical node relationships.

- [ ] "Create Portal" action: select two nodes -> generate PortalPair
- [ ] Auto-assign labels (A, B, C...) with matching glyph symbols
- [ ] Portal glyphs rendered as markers on both endpoint nodes
- [ ] Select and delete portal pairs
- [ ] Edit portal glyph symbols and colors
- [ ] Undo support for portal operations

**Verify**: Portals render as matching glyph markers, can be created and deleted

---

## Milestone 7: Text Editing

**Goal**: Edit node text content inline. Text changes modify the GlyphArea in the Baumhard tree, then sync back to the MindMap model.

### Session 7A: Inline text editor and node creation

**What**: Double-click to edit node text, create new nodes.

- [ ] Double-click node → enter edit mode with cursor (GlyphArea text mutation for cursor display)
- [ ] Text input, deletion, cursor movement, text selection
- [ ] Rich text: Ctrl+B bold, Ctrl+I italic (ColorFontRegion mutations)
- [ ] Esc or click outside → exit edit mode, sync text back to MindMap model
- [ ] Double-click empty space → create new root node (new GlyphArea in tree)
- [ ] Tab from selected node → create child node
- [ ] Enter from selected node → create sibling node

**Verify**: Text can be edited inline, new nodes can be created

---

## Milestone 8: Save & File Management

**Goal**: Persist changes, manage mindmap files.

### Session 8A: Save and file operations

**What**: Serialize changes to disk, basic file management.

- [ ] Serialize MindMap to `.mindmap.json` via serde (Ctrl+S)
- [ ] Auto-save on significant changes (debounced)
- [ ] Save confirmation on exit if unsaved changes
- [ ] Create new empty mindmap
- [ ] File picker for opening existing maps (native: `rfd` crate, WASM: file input)

**Verify**: Changes persist across app restarts, new maps can be created

---

## Verification Strategy

Each session should be verified by:

1. **Unit tests** - model operations in `lib/baumhard/src/mindmap/`
2. **Integration tests** - load test mindmaps, verify scene building
3. **Visual verification** - `cargo run` or `trunk serve`
4. **Existing tests** - `./test.sh` passes with no regressions

```bash
./test.sh                                    # All tests
cargo test -p baumhard --lib mindmap         # Mindmap module tests
cargo run -- maps/testament.mindmap.json     # Native app
trunk serve                                  # WASM app
```
