# Mandala Mindmap - Architecture & Roadmap

## Context

Mandala is a Rust mindmap application (~10,300 LOC) built on WebGPU (wgpu) with the
Baumhard glyph animation library. It currently renders a specific mindmap as read-only.
The app needs to evolve from a hardcoded viewer into a general-purpose, editable mindmap
tool inspired by miMind but with superior capabilities.

**Key drivers:**

1. The app currently loads one specific mindmap file - it should open any `.mindmap.json`
2. Application logic lives partly inside the Renderer - needs clean separation
3. The JSON format needs evolution to support multi-parent nodes, portals, and richer glyph-based visuals
4. No editing exists - users need intuitive node manipulation (move, reparent, connect, portal creation)
5. The 1:N relationship between mindmap nodes and GlyphElements must be formalized

---

## Architectural Decisions

1. **Single-threaded architecture** - Collapse multi-threaded Decree system to direct method calls. Simpler state management, matches WASM reality, better for editing UX
2. **Model-View separation** - MindMapDocument owns the MindMap, Renderer receives RenderScene snapshots
3. **RenderScene as intermediary** - Formalizes the 1:N mapping (one node -> text + border + connection elements)
4. **Multi-parent via Vec** - `parent_ids: Vec<String>` where [0] is primary, rest are secondary
5. **Portals as strict pairs** - PortalPair struct with exactly two endpoints, rendered as matching glyph markers (like circuit vias)
6. **Everything is glyphs** - Connections, borders, portals all rendered as positioned font glyphs via cosmic-text
7. **Game code removed** - All World/Scene/GameObject/GameResource types deleted. Clean mindmap-focused codebase
8. **Backward-compatible format** - v1 files load in v2 loader via serde defaults + migration

---

## Current Architecture Assessment

### What exists and works well
- **Baumhard library** (`lib/baumhard/`) - solid glyph rendering primitives, tree/mutation system
- **MindMap data model** (`lib/baumhard/src/mindmap/model.rs`) - clean serde-based structs
- **Border system** (`lib/baumhard/src/mindmap/border.rs`) - glyph-based borders with presets
- **JSON format v1.0** with v1.1 glyph extensions (GlyphBorderConfig, GlyphConnectionConfig)
- **Renderer** (`src/application/renderer.rs`) - working wgpu+glyphon pipeline with camera
- **Multi-target** - native + WASM builds via Trunk

### What needs change
- **Renderer owns MindMap** - `renderer.rs:548` stores `self.mindmap = Some(map)` and builds buffers directly
- **No editing layer** - no input handling for node selection, drag, connect
- **Connections not rendered** - MindEdge/GlyphConnectionConfig defined but render code absent
- **Single parent only** - `MindNode.parent_id: Option<String>` allows only one parent
- **No portal concept** in the format
- **Game engine leftovers** in `common.rs` and `game_concepts.rs`
- **Hardcoded load path** - no file picker or CLI arg

### Key Files Reference
| File | LOC | Role |
|------|-----|------|
| `src/application/app.rs` | 665 | Event loop, window, thread management |
| `src/application/renderer.rs` | 902 | GPU pipeline, mindmap buffer building |
| `src/application/common.rs` | 470 | Decree system + game leftovers |
| `src/application/game_concepts.rs` | 213 | Game state (to be removed) |
| `lib/baumhard/src/mindmap/model.rs` | 303 | MindMap, MindNode, MindEdge structs |
| `lib/baumhard/src/mindmap/loader.rs` | ~50 | JSON loading |
| `lib/baumhard/src/mindmap/border.rs` | 147 | BorderGlyphSet, BorderStyle |
| `lib/baumhard/src/gfx_structs/element.rs` | ~200 | GfxElement enum |
| `lib/baumhard/src/gfx_structs/tree_walker.rs` | 323 | Mutation tree walking |

---

## Milestone Dependency Graph

```
M1 (Architecture) --+--> M2 (Connections) ---------> M6 (Connection Editing)
                     |
                     +--> M3 (Format v2) -----------> M6 (Portal Creation)
                     |
                     +--> M4 (Selection) --> M5 (Move/Reparent) --> M7 (Text Edit)
                     |
                     +--> M8 (Save/File) [can start after M1]
```

---

## Milestone 1: Architecture Foundation

**Goal**: Clean separation of Model, Application Logic, and Rendering. Generic file loading.

### Session 1A: Remove game engine leftovers & simplify threading

**What**: Strip all game-specific code and collapse multi-threaded architecture.

- [ ] Delete `src/application/game_concepts.rs` entirely (World, Scene, GameObject, etc.)
- [ ] Delete `src/application/main_menu.rs`
- [ ] Remove from `common.rs`: all game types (GameResourceType, GameStatModifier, GameObjectFlags, GameItemProperty, GameResourceAspect, StatModifier, etc. - lines 284-405)
- [ ] Remove `Decree`/`Instruction`/`AckKey` channel infrastructure from `common.rs`
- [ ] Remove `HostDecree` game variants (MasterSound*, Load/Save/Pause/ExitInstance)
- [ ] Keep from `common.rs`: `WindowMode`, `InputMode`, `RedrawMode`, `KeyPress`, `StopWatch`, `PollTimer`
- [ ] Refactor `app.rs` to single-threaded: remove crossbeam channels, collapse event loop to own Renderer directly
- [ ] Ensure it compiles and renders the existing mindmap

**Verify**: `cargo build` succeeds, `cargo run` renders testament mindmap as before

### Session 1B: Introduce MindMapDocument and RenderScene

**What**: Clean Model-View separation with the Document and RenderScene abstractions.

- [ ] Create `src/application/document.rs` with `MindMapDocument` struct (owns `MindMap`, `SelectionState`, `dirty` flag, `undo_stack`)
- [ ] Move `self.mindmap` out of Renderer into Document
- [ ] Create `lib/baumhard/src/mindmap/scene_builder.rs` with `RenderScene` struct
- [ ] Extract scene-building logic from `renderer.rs:582-716` (`rebuild_mindmap_buffers`) into scene_builder
- [ ] `RenderScene` contains: `text_elements`, `border_elements` (connection/portal elements as empty Vecs for now)
- [ ] Renderer receives `&RenderScene` and builds cosmic-text buffers from it
- [ ] Application owns both Document and Renderer, wires them together

**Verify**: `cargo build` + `cargo run` renders same result, `cargo test` passes

### Session 1C: Generic file loading

**What**: Load any `.mindmap.json` from CLI args or WASM URL.

- [ ] Accept mindmap file path as CLI argument (`std::env::args`)
- [ ] Default to `maps/testament.mindmap.json` if no arg provided
- [ ] For WASM: read from URL query parameter or embedded default
- [ ] Application creates Document from loaded MindMap, passes to Renderer
- [ ] Test with multiple mindmap files

**Verify**: `cargo run -- maps/testament.mindmap.json` works, `cargo run -- other.mindmap.json` works

---

## Milestone 2: Connection Rendering

**Goal**: Render edges between nodes using glyph-based connections.

### Session 2A: Connection path computation

**What**: Compute paths between connected nodes and lay out glyphs along them.

- [ ] Create `lib/baumhard/src/mindmap/connection.rs`
- [ ] Implement path computation: source anchor -> target anchor (straight line)
- [ ] Add Bezier curve support using existing `control_points` data
- [ ] Sample points along path at intervals matching glyph spacing
- [ ] Define anchor point system: 0=auto, 1-4=top/bottom/left/right

**Verify**: Unit tests for path computation and point sampling

### Session 2B: Glyph connection rendering

**What**: Generate render elements for connections and display them.

- [ ] Add `ConnectionElement` to `RenderScene`
- [ ] Scene builder generates connection elements from `MindEdge` data
- [ ] Use `GlyphConnectionConfig.body` as repeating glyph (default: middle dot)
- [ ] Place `cap_start`/`cap_end` glyphs at endpoints
- [ ] Renderer builds cosmic-text buffers for connections
- [ ] Fall back to canvas `default_connection` when edge has no `glyph_connection`

**Verify**: Edges visible between nodes in testament mindmap

---

## Milestone 3: JSON Format v2

**Goal**: Evolve the format to support multi-parent, portals, and richer glyph semantics.

### Session 3A: Multi-parent support

**What**: Change node parent model from single to multi-parent.

- [ ] Change `MindNode.parent_id: Option<String>` to `parent_ids: Vec<String>`
- [ ] Implement serde backward compatibility: accept both `parent_id` (v1) and `parent_ids` (v2)
- [ ] Update `root_nodes()` - nodes with empty `parent_ids`
- [ ] Update `children_of()` - check if `parent_ids` contains the given parent
- [ ] Update `is_hidden_by_fold()` - fold based on primary parent `[0]` only
- [ ] Update `find_schema_root()` - walk primary parent chain
- [ ] Update existing tests, add multi-parent tests

**Verify**: Existing v1 mindmap loads unchanged, new multi-parent JSON loads correctly

### Session 3B: Portal system & format v2 spec

**What**: Add portal data model and document the v2 format.

- [ ] Add `PortalPair`, `PortalEndpoint`, `PortalGlyph` structs to `model.rs`
- [ ] Add `portals: Vec<PortalPair>` to `MindMap` with `#[serde(default)]`
- [ ] Add `AnchorPosition` enum (Auto, Top, Bottom, Left, Right, Custom)
- [ ] Implement v1 -> v2 migration in loader (parent_id -> parent_ids, add empty portals)
- [ ] Bump format to `"version": "2.0"`
- [ ] Update `maps/docs/mindmap-json-format.md` with v2 spec
- [ ] Add portal rendering elements to scene builder (glyph markers at anchor points)

**Verify**: v1 and v2 files both load, portal glyphs render on nodes

---

## Milestone 4: Selection & Basic Interaction

**Goal**: Users can select, highlight, and inspect nodes.

### Session 4A: Hit testing and selection

**What**: Click nodes to select them, with visual feedback.

- [ ] Add `screen_to_canvas()` method to Camera2D (inverse of `canvas_to_screen`)
- [ ] Implement node hit testing: click position -> find node by bounds
- [ ] Add `SelectionState` enum to Document (None, Single, Multi, Edge, Portal)
- [ ] Handle click events in app event loop -> delegate to Document
- [ ] Shift+click for multi-select, click empty space to deselect
- [ ] Renderer draws highlight border/glow on selected nodes

**Verify**: Clicking nodes selects them with visual feedback

---

## Milestone 5: Node Editing - Move & Reparent

**Goal**: Drag nodes to reposition, reparent via drag-and-drop.

### Session 5A: Node movement

**What**: Drag to move nodes, with subtree and individual modes.

- [ ] Implement drag gesture detection (mouse down -> move -> mouse up)
- [ ] Default drag: move node + all descendants (translate subtree)
- [ ] Long press + drag: move individual node only
- [ ] Update positions in MindMap model, mark document dirty
- [ ] Rebuild affected render elements on change
- [ ] Add undo support: push `MoveAction` to undo stack

**Verify**: Nodes can be dragged around, subtrees move together

### Session 5B: Reparent via drag

**What**: Drag a node onto another to reparent it.

- [ ] Detect drag-over-node: highlight potential parent target
- [ ] On release over node: reparent (update `parent_ids[0]`, recalculate index)
- [ ] Support drop as first child, last child, or between siblings
- [ ] Visual indicators for drop position
- [ ] Undo support for reparent operations

**Verify**: Nodes can be reparented by dragging, undo restores previous state

---

## Milestone 6: Connection Editing

**Goal**: Create, delete, and modify connections between nodes.

### Session 6A: Create and delete connections

**What**: Draw connections between nodes and delete existing ones.

- [ ] "Connect mode": select source node, click target to create edge
- [ ] Visual feedback: temporary connection following cursor
- [ ] Create new `MindEdge` with default glyph style
- [ ] Support edge types: `parent_child`, `cross_link`, `arbitrary`
- [ ] Multi-parent: connecting as additional parent adds to `parent_ids`
- [ ] Delete selected connection (delete key or context action)

**Verify**: New connections can be created and deleted

### Session 6B: Portal creation and connection manipulation

**What**: Create portal pairs and edit connection paths.

- [ ] "Create Portal" action: select two nodes -> generate PortalPair
- [ ] Auto-assign labels (A, B, C...) with matching glyph symbols
- [ ] Drag control points to curve existing connections
- [ ] Change connection style (glyph, color, width) via selection

**Verify**: Portals render as matching glyph markers, connections can be curved

---

## Milestone 7: Text Editing

**Goal**: Edit node text content inline.

### Session 7A: Inline text editor and node creation

**What**: Double-click to edit node text, create new nodes.

- [ ] Double-click node -> enter edit mode with cursor
- [ ] Text input, deletion, cursor movement, text selection
- [ ] Rich text: Ctrl+B bold, Ctrl+I italic
- [ ] Esc or click outside -> exit edit mode, save to model
- [ ] Double-click empty space -> create new node
- [ ] Tab from selected node -> create child node
- [ ] Enter from selected node -> create sibling node

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
