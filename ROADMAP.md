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

## Current State

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
- **Node movement** — drag selected nodes to reposition (subtree moves together), alt+drag for individual node only. Mutations applied to Baumhard tree for real-time preview, model synced on drop. Connections/borders snap on drop.
- **Reparent via hotkey mode** — Ctrl+P on selected nodes enters reparent mode (orange highlight), click target node to attach as last children (or click empty canvas to promote to root), Esc to cancel. Cycle prevention built in. Green hover preview on drop target.
- **Undo** — Ctrl+Z undoes node movement, reparent, edge creation/deletion, orphan creation, custom mutations, and canvas (theme) snapshots
- **DragState machine** — structured input state (Pending/Panning/MovingNode) replaces loose booleans
- **AppMode** — high-level mode enum (Normal / Reparent / Connect) cleanly separates mode-based flows from the regular drag flow
- **Connection selection** — click on a connection path (point-to-path hit test, zoom-aware 8px tolerance) to select it; mutually exclusive with node selection. Selected edges render in cyan via scene-builder color override.
- **Connection deletion** — Delete key removes the selected edge. `UndoAction::DeleteEdge { index, edge }` restores it at the original index on Ctrl+Z.
- **Connection creation** — Ctrl+D with one selected node enters connect mode (source orange, hovered target green, reusing the reparent color scheme), click target to create a `cross_link` edge. Auto-selects the newly-created edge. Ctrl+Z undoes. Esc cancels. Self-links / duplicates / unknown nodes are silent no-ops.
- **Orphan node creation** — Ctrl+N creates a new unattached node at the cursor position with placeholder text. `parent_id = None` so it starts as a root; reparent mode (Ctrl+P) can then attach it once the user is ready. Ctrl+Z undoes the creation.
- **Orphan selection (detach)** — Ctrl+O severs the selected node(s) from their parent, promoting them to root. Each detached node's entire subtree stays attached to it. Ctrl+Z undoes, restoring the original parent link and index.
- **Configurable keybindings** — every keyboard action is reconfigurable via a JSON config file. Desktop loads from `--keybinds <path>` CLI flag or a conventional path (`$XDG_CONFIG_HOME/mandala/keybinds.json`, else `$HOME/.config/mandala/keybinds.json`). WASM loads from a `?keybinds=<url-encoded-json>` query param or `localStorage['mandala_keybinds']`. Missing fields fall back to hardcoded defaults so partial configs work. Invalid bindings are logged and skipped; the app never crashes for a bad keybinds file. Modifier aliases (cmd/command/meta/super → Ctrl, option → Alt) make the same config work across platforms. Sample config in `config/default_keybinds.json`.
- **Custom mutation scaffolding** — types and registry machinery exist in code from earlier tangent work: `CustomMutation` carries a bundle of `Mutation` operations plus a `TargetScope`, a `MutationBehavior`, and an optional `Predicate`. `MindMapDocument` maintains a `mutation_registry` merged from map-level and inline-on-node definitions, with `find_triggered_mutations()` and `apply_custom_mutation()` wiring the tree walk and model sync. `UndoAction::CustomMutation { node_snapshots }` captures pre-mutation state for Ctrl+Z. M9 is not officially complete — hover/key dispatch, global config loading, and the proper demo map remain open.
- **Click-triggered mutations (partial)** — `handle_click` dispatches `OnClick` trigger bindings via `find_triggered_mutations` + `apply_custom_mutation`, filtered by `PlatformContext::Desktop`. Rootless nodes with trigger bindings become user-defined "buttons" that fire custom mutations and/or document actions on click. Hover and key triggers are still stubbed.
- **Theme variables (CSS custom properties)** — `Canvas.theme_variables: HashMap<String, String>` holds the live variable map (e.g. `"--bg" → "#141414"`). Any color field in the map can reference a variable via `var(--name)` and it is resolved at scene-build time through `resolve_var()` in `util/color.rs`. Unknown variables and malformed references pass through unchanged (graceful fallback); `hex_to_rgba_safe()` provides a non-panicking hex parser so a theme typo can't crash the render. Resolution sites cover background, frame color, connection/edge color, and per-run text colors in both the flat scene pipeline and the Baumhard tree pipeline.
- **Theme variants** — `Canvas.theme_variants: HashMap<String, HashMap<String, String>>` stores named presets (`"light"`, `"dark"`, `"forest"`). Activating a variant copies its map into the live `theme_variables`; presets themselves are pure authoring state and never referenced at render time. A single source of truth means Ctrl+S saves whatever the user last switched to.
- **Document actions** — `CustomMutation.document_actions: Vec<DocumentAction>` carries canvas/document-level effects alongside the per-node mutations. `DocumentAction::SetThemeVariant(name)` swaps in a named preset; `SetThemeVariables(map)` patches the live variables ad-hoc. Applied by `MindMapDocument::apply_document_actions()`, which snapshots the full canvas into `UndoAction::CanvasSnapshot` before any change, so Ctrl+Z restores a theme swap in one hop.
- **Button-node cursor polish** — hovering a node with any non-empty `trigger_bindings` switches the cursor to `CursorIcon::Pointer`. Transitions are tracked so `set_cursor` only fires when the hover state actually changes. Native only — WASM input path remains a known gap.
- **Stress-test map generator** — `cargo run -p baumhard --bin generate_stress_map -- ...` writes synthetic `.mindmap.json` files of arbitrary size and topology (balanced / skewed / star), with `--long-edges K` to insert deliberately far-apart cross-links for connection-render perf testing and `--seed` for deterministic output. The smoke-test rig for Phase 4 of the theme-variables tangent.
- **Mutation-frequency throttle** — under load, the `AboutToWait` drag path drains its accumulated `pending_delta` every Nth frame instead of every frame, where N is a moving-average-driven self-tuning multiplier that holds per-frame work time under the screen refresh budget. Input accumulation stays snappy (every mouse event still folds into `pending_delta`); the dragged node advances in chunks under stress, catching up to the cursor every N frames. Healthy load = N = 1 (no throttling). Implements the governing-invariant half of the connection/border render-cost work.
- **Viewport culling on connection glyphs** — `rebuild_connection_buffers` now computes the visible canvas rect once per call (with a `font_size` margin on each side) and skips cosmic-text buffer creation for any glyph position outside it. The dominant per-frame cost in the connection rebuild path is cosmic-text shaping; for a long cross-link most of its sample positions are off-screen during drag, so skipping those drops per-frame work by ~48× in the user's long-connection stutter scenario without changing visible output.
- **Keyed incremental rebuild for connections and borders** — Phase 4(B) of the connection-render cost work. Two targeted caches keyed on stable identity (edges: `(from_id, to_id, edge_type)` via `SceneConnectionCache` in `lib/baumhard/src/mindmap/scene_cache.rs`; nodes: `id`). During drag, `build_scene_with_cache` skips `sample_path` + control-point/Bezier work for edges whose endpoints did not move — the expensive per-frame cost that Phase A could not touch — and the `Renderer` keeps keyed `HashMap`s for `border_buffers` and `connection_buffers` so unmoved entries reuse their shaped cosmic-text buffers across frames, patched in place via `pos` only. The clip filter (`point_inside_any_node`) still re-runs against the current frame's node AABBs for cached edges, so a stable long cross-link still clips correctly around a moved-but-unrelated third node (governing invariant preserved). Selection changes apply the highlight color override at read time, so flipping selection doesn't invalidate the cache. Camera pan/zoom clears only the renderer-side connection map (viewport cull output changes) while leaving the document-side geometry cache intact. Each drag starts with a fresh cache to handle inter-drag structural edits; the first drag frame is a full rebuild and subsequent frames are incremental. Eliminates the ~1,700 bezier evaluations / frame / long edge that Phase A left on the table — the upstream geometry cost, not just the downstream shaping cost.
- **Edge reshape via grab-handles** — Session 6C. When a connection is selected, the scene builder emits small cyan `◆` handles at both anchor endpoints and either (a) the midpoint of straight edges or (b) each stored control point of curved edges. Clicking a handle and dragging past the 5 px threshold enters `DragState::DraggingEdgeHandle`, which per-frame drains the accumulated cursor delta into the model edge in place: control-point handles write an `(x, y)` offset from the relevant node center, anchor handles snap to whichever side (top/right/bottom/left) of the node is closest to the cursor, and the straight-edge midpoint handle inserts a fresh control point on first drain and promotes itself to `ControlPoint(0)` for subsequent frames — the "curve a straight line" gesture. `scene_cache.invalidate_edge` is called per frame for the single dirty edge, keeping the Phase B incremental rebuild path hot for everything else. On release, the pre-drag snapshot is pushed as `UndoAction::EditEdge { index, before }`, so Ctrl+Z restores any reshape in one hop.
- **Command palette** — Session 6C. Press `/` to open a glyph-rendered modal listing context-aware actions for the current selection. Type to fuzzy-filter (case-insensitive subsequence with a word-boundary bonus), Up/Down navigates, Enter executes, Esc / click-outside closes. While open, the palette steals all keyboard input so existing hotkeys don't fire. The global action registry is a static slice of `PaletteAction { id, label, description, tags, applicable, execute }` in `src/application/palette.rs`; Session 6C ships eleven entries — "Reset connection to straight" (hidden by its applicability predicate for edges that are already straight) and two groups of five anchor setters ("Set from-anchor: Auto/Top/Right/Bottom/Left", same for to-anchor). Each action's execute mutates the document through the new `reset_edge_to_straight` / `set_edge_anchor` helpers, which snapshot the edge and push `UndoAction::EditEdge` so undo works identically to the drag-handle path. The palette overlay renders as cosmic-text buffers in screen coordinates above everything else.
- **Multi-target** — native + WASM builds
- **366 tests passing**

### What needs work
- **No text editing** — no inline text editing or node creation
- **Borders not in tree** — borders render via flat pipeline, not as GlyphModel children in the Baumhard tree
- **No save/persistence** — document dirty flag exists but no serialization
- **Hover and key trigger dispatch** — `OnHover` and `OnKey` triggers are in the data model but the event loop only dispatches `OnClick` so far
- **Global custom mutations** — `~/.mandala/custom_mutations.json` loading is not yet wired
- **WASM input** — selection, movement, reparent, trigger dispatch not yet wired up on WASM (TODOs in event loop)
- **Animated mutations** — mutations currently apply instantly; the timing/interpolation layer planned in Session 10B is not yet built
- **Keyboard map navigation** — no keyboard pan/zoom; mouse-driven only
- **Fast-pan stutter** — model mutation work can fall behind input during very fast drags

### Key Files Reference
| File | Role |
|------|------|
| `src/application/app.rs` | Event loop, window, DragState machine, input handling, tree+scene pipeline wiring |
| `src/application/renderer.rs` | GPU pipeline: tree-based node rendering + flat border/connection rendering |
| `src/application/document.rs` | Owns MindMap + SelectionState + UndoStack, provides `build_tree()`, `build_scene()`, `hit_test()`, `apply_selection_highlight()`, `apply_drag_delta()`, `apply_move_subtree/single()`, `undo()`, `apply_custom_mutation()`, `apply_document_actions()` |
| `src/application/common.rs` | RenderDecree, WindowMode, InputMode, timing |
| `src/application/frame_throttle.rs` | `MutationFrequencyThrottle` — the governing-invariant safety net for the drag path |
| `src/application/keybinds.rs` | Configurable key-to-Action mapping with layered loading |
| `lib/baumhard/src/mindmap/model.rs` | MindMap, MindNode, MindEdge, Canvas (incl. `theme_variables`, `theme_variants`), `all_descendants()` |
| `lib/baumhard/src/mindmap/tree_builder.rs` | MindMap → Tree<GfxElement, GfxMutator> bridge, resolves text-run colors through theme variables |
| `lib/baumhard/src/mindmap/scene_builder.rs` | MindMap → flat RenderScene (connections, borders, background), resolves colors through theme variables. Phase B: `build_scene_with_cache` reuses cached per-edge sample geometry |
| `lib/baumhard/src/mindmap/scene_cache.rs` | `SceneConnectionCache`, `EdgeKey`, `CachedConnection` — Phase B per-edge pre-clip sample cache with `by_node` reverse index |
| `lib/baumhard/src/mindmap/loader.rs` | JSON loading |
| `lib/baumhard/src/mindmap/border.rs` | BorderGlyphSet, BorderStyle |
| `lib/baumhard/src/mindmap/connection.rs` | Path computation, Bezier curves, glyph sampling |
| `lib/baumhard/src/mindmap/custom_mutation.rs` | `CustomMutation`, `TargetScope`, `MutationBehavior`, `Trigger`, `TriggerBinding`, `DocumentAction`, `PlatformContext` |
| `lib/baumhard/src/util/color.rs` | Hex parsing, `Color` arithmetic, `resolve_var()`, `hex_to_rgba_safe()` |
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
                       |                          |      includes M5.1 (Live Connections During Drag)
                       |                          |
                       |                          +--> M7 (Text Edit)
                       |                          |      via GlyphArea text mutations
                       |                          |
                       |                          +--> M9 (Custom Mutations)
                       |                                 user-defined mutations with triggers
                       |                                 toggle + persistent behaviors
                       |                                 platform-context-aware dispatch
                       |
                       +--> M8 (Save/File) [can start any time]
                              M8 enables M9 persistence (Ctrl+S saves custom mutation effects)
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

**Verify**: `cargo test -p baumhard -p mandala` passes (126 tests at end of session 4), clicking nodes selects them with visual feedback

---

## Milestone 5: Node Editing - Move & Reparent

**Goal**: Drag nodes to reposition, reparent via drag-and-drop. Movement expressed as position mutations on the Baumhard tree.

### Session 5A: Node movement

**What**: Drag to move nodes, with subtree and individual modes.

- [x] Implement drag gesture detection (mouse down → move → mouse up) via DragState enum (None/Pending/Panning/MovingNode)
- [x] Default drag: move node + all descendants — in-place GlyphArea mutations for real-time preview, model sync on drop
- [x] Alt+drag: move individual node only (apply mutation to single node, not descendants)
- [x] Update positions in MindMap model on drop, mark document dirty
- [x] Full rebuild from model on drop (tree + connections + borders)
- [x] Add undo support: UndoAction::MoveNodes with original positions, Ctrl+Z to undo
- [x] 9 new tests: all_descendants, move_subtree, move_single, move_preserves_relative, undo_restores, drag_delta, drag_delta_with_descendants, move_returns_originals

**Key files**:
- `src/application/app.rs` — DragState enum, refactored input handling, alt/ctrl tracking, Ctrl+Z undo
- `src/application/document.rs` — UndoAction, undo_stack, apply_move_subtree/single, undo(), apply_drag_delta()
- `lib/baumhard/src/mindmap/model.rs` — all_descendants() helper

**Verify**: `cargo test -p baumhard -p mandala` passes (135 tests), nodes can be dragged around, subtrees move together

### Session 5B: Reparent via hotkey mode (Ctrl+P)

**What**: Explicit reparent mode — press Ctrl+P with one or more nodes selected
to enter "reparent mode," then click a target node to attach the source(s) as its
last children. Reparenting is not an implicit drag gesture but an explicit,
cancellable mode with clear visual feedback.

- [x] `AppMode::Reparent { sources }` enum tracks the mode in the event loop
- [x] Ctrl+P enters the mode when ≥1 node is selected; Esc cancels
- [x] In reparent mode: left-click on a node reparents sources under it as last
      children; left-click on empty canvas promotes sources to root
- [x] Cycle prevention: silently skip invalid targets (target is self or descendant of source) via new `MindMap::is_ancestor_or_self()` helper
- [x] Source nodes highlighted orange during mode; hovered target highlighted green
- [x] Tree + scene rebuilt after reparent (tree structure changed)
- [x] `UndoAction::ReparentNodes { entries }` records `(id, old_parent_id, old_index)` for every reparented node; Ctrl+Z restores
- [x] Drop position: new nodes become last children (`index = max(siblings.index) + 1`)
- [x] Multi-select: all selected nodes become siblings under the new parent, preserving argument order
- [x] 11 new tests (5 `is_ancestor_or_self` + 6 reparent/undo)

**Key files**:
- `lib/baumhard/src/mindmap/model.rs` — `is_ancestor_or_self()` helper
- `src/application/document.rs` — `UndoAction::ReparentNodes`, `apply_reparent()`, `apply_reparent_source_highlight()`, `apply_reparent_target_highlight()`, extended `undo()`
- `src/application/app.rs` — `AppMode` enum, `app_mode`/`hovered_node` state, Ctrl+P/Esc handling, left-click reroute in reparent mode, `handle_reparent_target_click()` and `rebuild_all_with_mode()` helpers

**Verify**: `cargo test -p baumhard -p mandala` passes (172 tests). Select a node, Ctrl+P (turns orange), hover another node (turns green), click to reparent. Ctrl+Z undoes. Esc cancels. Click empty canvas in mode promotes to root.

### Session 5.1: Live Connections & Borders During Drag

**What**: Connections and borders update in real-time during node drag instead of only on mouse release.

- [x] Add `build_scene_with_offsets(map, offsets: &HashMap<&str, (f32, f32)>)` to scene_builder — applies position deltas when reading node positions for connections and borders
- [x] Refactor `build_scene()` to call `build_scene_with_offsets` with an empty map
- [x] Add `build_scene_with_offsets()` to MindMapDocument
- [x] In `app.rs` `AboutToWait` / `MovingNode` branch: compute offset map from `node_ids` + `total_delta`, rebuild connection + border buffers each frame during drag

**Key files**:
- `lib/baumhard/src/mindmap/scene_builder.rs` — `build_scene_with_offsets()`
- `src/application/document.rs` — forwarding method
- `src/application/app.rs` — wire into drag frame loop

**Verify**: Drag a node — connections and borders follow smoothly in real-time

---

## Milestone 6: Connection Editing

**Goal**: Create, delete, and fully customize connections between nodes.

### Session 6A: Connection selection and deletion

**What**: Select connections and delete them, with visual feedback.

- [x] Click on a connection glyph path to select the edge (falls through from
      node hit test when the cursor misses every node)
- [x] Hit testing for connections: point-to-path distance via new
      `connection::distance_to_path()`, scaled by zoom so 8 screen pixels of
      click tolerance stay visually stable across zoom levels
- [x] Visual feedback: selected connection color-overridden to cyan
      (`#00E5FF`) via `scene_builder::build_scene_with_offsets_and_selection`
- [x] `SelectionState::Edge(EdgeRef)` variant; node and edge selection are
      mutually exclusive
- [x] Delete key removes the selected edge
- [x] Undo support: `UndoAction::DeleteEdge { index, edge }` restores the
      edge at its original index
- [x] 22 new tests added (7 `distance_to_path` + 15 document-level)

**Key files**:
- `lib/baumhard/src/mindmap/connection.rs` — `distance_to_path()` + tests
- `lib/baumhard/src/mindmap/scene_builder.rs` — selection-aware scene build
- `src/application/document.rs` — `EdgeRef`, `SelectionState::Edge`,
  `hit_test_edge()`, `remove_edge()`, `build_scene_with_selection()`,
  `UndoAction::DeleteEdge`/`CreateEdge`
- `src/application/renderer.rs` — `canvas_per_pixel()` helper for tolerance
  conversion
- `src/application/app.rs` — edge hit test wired into `handle_click`, Delete
  key handler, `rebuild_all` uses selection-aware scene build

**Verify**: Click a connection to select it, press Delete to remove it,
Ctrl+Z to undo. `./test.sh` passes (196 tests).

### Session 6B: Create connections

**What**: Draw new connections between nodes.

- [x] Connect mode via Ctrl+D ("D for draw"): requires exactly one node
      selected, enters `AppMode::Connect { source }`, next left-click on a
      target node creates the edge
- [x] Visual feedback: source node orange, hovered target node green —
      reuses the existing reparent-mode color scheme
- [x] New `MindEdge` created with `default_cross_link_edge()` (color
      `#aa88cc`, width 3, auto anchors, no control points)
- [x] Cross-links are arbitrary connections — they don't affect the tree
      hierarchy. Hierarchy edges continue to be created via Ctrl+P reparent.
- [x] Duplicate/self-link/unknown-node guards return silent no-ops
- [x] Undo support: `UndoAction::CreateEdge { index }` pops the created edge
- [x] Newly-created edge is auto-selected so the user gets immediate visual
      confirmation and can Delete or style it next
- [x] Esc cancels connect mode without side effect
- [x] 6 new tests covering creation success, self-link rejection, duplicate
      rejection, unknown-node rejection, default-field correctness, undo

**Key files**:
- `src/application/document.rs` — `create_cross_link_edge()`,
  `default_cross_link_edge()`, `UndoAction::CreateEdge`
- `src/application/app.rs` — `AppMode::Connect`, Ctrl+D hotkey,
  `handle_connect_target_click()`, extended `rebuild_all_with_mode` and
  cursor-move hover tracking

**Verify**: Select a single node, Ctrl+D (source turns orange), hover other
nodes (green preview), click to create a cross-link edge. Ctrl+Z undoes.
Esc cancels. `./test.sh` passes (196 tests total at end of Session 6A+B).

### Session 6C: Connection path manipulation + command palette

**What**: Edit connection paths, control points, and anchor positions
via direct manipulation (grab-handles on the selected edge), plus a
new context-aware command palette that hosts long-tail actions
without burning a hotkey per action. The palette is the UI answer to
"we can't have a single shortcut for every action" — existing hotkeys
stay, and new actions land in the palette instead.

- [x] Drag control points to curve existing straight connections (adds Bezier control points via the "Midpoint" handle gesture)
- [x] Drag existing control points to reshape curved connections
- [x] Visual handles: render draggable control-point markers on selected connections (cyan `◆` glyphs at anchors, CPs, and a midpoint for straight edges)
- [x] Change anchor points: drag connection endpoints to different sides of a node (top/right/bottom/left); during drag, the anchor snaps to whichever side midpoint is closest to the cursor
- [x] Snap anchor to nearest edge midpoint on release (falls out of the drag path above — the stored value is always one of 1..=4)
- [x] Reset connection to straight line via command palette action ("Reset connection to straight"), applicability predicate hides it for already-straight edges
- [x] Undo support for all path modifications via new `UndoAction::EditEdge { index, before }` variant; handle drags, palette reset, and palette anchor changes all use it
- [x] `DragState::DraggingEdgeHandle` variant with `Pending`→handle-hit precedence over node hits; per-frame drain loop mirrors `MovingNode`, writes model in place, invalidates only the dirty edge in the scene cache
- [x] Command palette infrastructure in `src/application/palette.rs`: `PaletteAction` (id, label, description, tags, applicable, execute), `PaletteContext`/`PaletteEffects`, `fuzzy_score` subsequence matcher with word-boundary bonus, `filter_actions` sorted by score descending
- [x] Eleven initial actions: "Reset connection to straight" + "Set from-anchor: Auto/Top/Right/Bottom/Left" + "Set to-anchor: Auto/Top/Right/Bottom/Left"
- [x] Palette UI: `/` opens it, glyph-rendered box frame (reuses box-drawing border chars) with a `/query▌` input line and one row per filtered action, selected row prefixed with `▸` in cyan; Up/Down navigates, Enter executes, Esc or any click outside closes; steals all keyboard input while open so Ctrl+Z etc. don't leak through
- [x] 68 new tests (13 handle-drag + edge edit helpers + undo, 5 scene-builder handle emission, 11 palette fuzzy/registry/applicability, 39 covering the broader surface)

**Key files**:
- `lib/baumhard/src/mindmap/scene_builder.rs` — `EdgeHandleElement`, `EdgeHandleKind`, `build_edge_handles`, `RenderScene::edge_handles` field, emission inside `build_scene_with_cache` when `selected_edge` is `Some`
- `src/application/document.rs` — `hit_test_edge_handle`, `reset_edge_to_straight`, `set_edge_anchor`, `edge_index`, `UndoAction::EditEdge` + undo handler
- `src/application/renderer.rs` — `edge_handle_buffers` field + `rebuild_edge_handle_buffers`; `palette_overlay_buffers` + `rebuild_palette_overlay_buffers` with `PaletteOverlayGeometry`/`PaletteOverlayRow`; both wired into the render pass
- `src/application/app.rs` — `DragState::DraggingEdgeHandle`, extended `Pending` with `hit_edge_handle`, `apply_edge_handle_drag` helper, `nearest_anchor_side` snap helper, `PaletteState` enum, `handle_palette_key` + `rebuild_palette_overlay`, `/` opening, click-outside-to-close
- `src/application/palette.rs` — **new** module with the `PaletteAction` registry, fuzzy-match, filter, and 11 Session 6C actions
- `src/application/mod.rs` — expose the new `palette` module

**Verify**: `./test.sh` green (366 tests, +68 new). `cargo run -- maps/testament.mindmap.json`, select a connection, drag its midpoint handle to curve it, drag a control-point handle to reshape, drag an anchor to snap to a different side. Press `/`, type "reset", Enter — the edge returns to straight. Press `/`, type "from top", Enter — source anchor snaps to the top. Ctrl+Z after any of the above restores the previous edge state.

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

## Milestone 9: Custom Mutations

**Goal**: Users can define named, reusable mutations and attach them to nodes with configurable triggers. Mutations cascade through the tree using the existing Baumhard mutation engine (tree walker, channels, predicates, macros). Both persistent (model-syncing, saveable) and toggle (visual-only, reversible) behaviors are supported. Triggers are platform-context-aware (Desktop/Web/Touch).

### Session 9A: Data model and serialization

**What**: Define all custom mutation types, add fields to MindMap/MindNode, ensure backward-compatible JSON loading.

- [ ] Create `lib/baumhard/src/mindmap/custom_mutation.rs` with `CustomMutation`, `TargetScope`, `MutationBehavior`, `Trigger`, `TriggerBinding`, `PlatformContext`
- [ ] Add `pub mod custom_mutation` to `lib/baumhard/src/mindmap/mod.rs`
- [ ] Add `custom_mutations: Vec<CustomMutation>` to `MindMap` (serde default, skip_serializing_if empty)
- [ ] Add `trigger_bindings: Vec<TriggerBinding>` and `inline_mutations: Vec<CustomMutation>` to `MindNode` (serde default)
- [ ] Add `always_match: bool` field to `Predicate` with serde default false, update `test()` to short-circuit
- [ ] Round-trip serialization tests, backward compatibility tests

**Key types**:
- `CustomMutation` — id, name, `Vec<Mutation>`, `TargetScope`, `MutationBehavior`, optional `Predicate`
- `MutationBehavior` — `Persistent` (default, syncs to model) | `Toggle` (visual-only, reversible)
- `TargetScope` — `SelfOnly` | `Children` | `Descendants` | `SelfAndDescendants` | `Parent` | `Siblings`
- `Trigger` — `OnClick` | `OnHover` | `OnKey(String)` | `OnLink(String)`
- `TriggerBinding` — trigger + mutation_id + optional `Vec<PlatformContext>` filter
- `PlatformContext` — `Desktop` | `Web` | `Touch`

**Verify**: `cargo test` passes, existing testament map loads unchanged

### Session 9B: Mutator tree builder and application

**What**: Build MutatorTrees from CustomMutation definitions, apply via tree walker, sync to model.

- [ ] Implement `build_mutator_tree(custom) -> MutatorTree<GfxMutator>` for each TargetScope
- [ ] Add `mutation_registry: HashMap<String, CustomMutation>` to MindMapDocument
- [ ] Implement `build_mutation_registry()` — merges global + map + inline mutations
- [ ] Implement `find_triggered_mutations(node_id, trigger) -> Vec<&CustomMutation>`
- [ ] Implement `apply_custom_mutation()` — applies to tree, syncs to model for Persistent, tracks active toggles for Toggle
- [ ] Add `UndoAction::CustomMutation` variant with node snapshots
- [ ] Tests for each TargetScope, registry build, trigger matching, undo

**Verify**: `cargo test` passes, custom mutations apply correctly to tree and model

### Session 9C: Event loop trigger dispatch

**What**: Wire trigger dispatch into the app event loop for click, hover, and keyboard.

- [ ] Add `hovered_node: Option<String>` and `platform_context: PlatformContext` to event loop state
- [ ] In `handle_click()`: after selection update, fire OnClick triggers for clicked node
- [ ] In `CursorMoved` (DragState::None): hit test for hover, fire OnHover on node enter, reverse toggle on leave
- [ ] In `KeyboardInput`: check selected nodes for OnKey triggers matching pressed key
- [ ] Platform context filtering: skip triggers whose `contexts` list doesn't include current platform
- [ ] OnLink trigger: data model ready, dispatch deferred to M7 (text editing)

**Verify**: `cargo test` passes, triggers fire visually with demo map

### Session 9D: Global config and demo map

**What**: Load global custom mutations from config file, create demo mindmap.

- [ ] Load global mutations from `~/.mandala/custom_mutations.json` at startup
- [ ] Merge into registry with lowest precedence (global < map < inline)
- [ ] Create `maps/custom_mutations_demo.mindmap.json` with example triggers and mutations
- [ ] WASM platform detection via `matchMedia("(hover: hover)")` for Touch vs Web context

**Verify**: `cargo test` passes, demo map demonstrates click/hover/key triggers

---

## Tangent: Theme Variables & Map Customization

**Context**: Not a scheduled milestone — grew out of a request to make
maps feel like a place users own, not a file they edit. Users wanted to
author light/dark themes, build clickable buttons inside the map that
switch themes, eventually script smooth animated transitions, navigate
the canvas fluidly from the keyboard, and stress the app with very
large generated maps to keep the feel honest at scale. The work below
is non-linear — pieces can ship in any order. Only what is actually
done is checked off; the rest is a sketch of adjacent ground.

Full planning notes lived in the session plan file; this section is
the durable record of what landed and what's still open.

### Theme variables, variants, and click-triggered theme switching

**What**: CSS-custom-property-style variables on the canvas, referenced
via `var(--name)` anywhere a color string is accepted. Named presets
in a sibling dictionary, switched by copy-into-live. A new
`DocumentAction` enum on `CustomMutation` carries canvas-level effects
alongside the existing per-node mutations. `handle_click` fires
`OnClick` trigger bindings so rootless nodes become user-defined
buttons.

- [x] `Canvas.theme_variables: HashMap<String, String>` — the live
      variable map, serde-default so existing maps load unchanged
- [x] `Canvas.theme_variants: HashMap<String, HashMap<String, String>>`
      — named presets, pure authoring state, activated by copy into
      the live map
- [x] `resolve_var(raw, vars) -> &str` in `lib/baumhard/src/util/color.rs`
      — single-level indirection, unknown references pass through
      unchanged (graceful fallback)
- [x] `hex_to_rgba_safe(color, fallback) -> [f32; 4]` — non-panicking
      hex parse so a theme typo can't crash the render
- [x] `var(--name)` resolution wired into `scene_builder.rs` for
      background, frame color, connection/edge color, per-text-run
      color, and `tree_builder.rs` for text runs into the Baumhard
      tree (both rendering pipelines covered)
- [x] `DocumentAction::SetThemeVariant(name)` — copies a named preset
      into the live variable map
- [x] `DocumentAction::SetThemeVariables(map)` — ad-hoc patch of the
      live variable map
- [x] `CustomMutation.document_actions: Vec<DocumentAction>` — new
      field, non-breaking via `#[serde(default)]`, round-trips cleanly
- [x] `UndoAction::CanvasSnapshot { canvas }` — snapshots the full
      canvas before any document action mutates it; Ctrl+Z restores a
      theme swap in one hop
- [x] `MindMapDocument::apply_document_actions(custom) -> bool` —
      applies the actions, pushes the undo snapshot only when
      something actually changed
- [x] Click dispatch in `handle_click` — looks up OnClick triggers via
      `find_triggered_mutations`, runs `apply_custom_mutation` for node
      mutations and `apply_document_actions` for canvas effects,
      filtered by `PlatformContext::Desktop`. Native only.
- [x] Hand cursor (`CursorIcon::Pointer`) over nodes with non-empty
      `trigger_bindings`, tracked on a transition so `set_cursor` only
      fires when hover state actually changes
- [x] Demo map `maps/theme_demo.mindmap.json` — titled root, two
      styled children using `var(--...)` everywhere, three rootless
      button nodes (`[ dark ]`, `[ light ]`, `[ forest ]`) wired to
      `SetThemeVariant` document actions
- [x] 13 new tests: `var` resolution hit/miss/malformed/whitespace/
      no-recursion, lenient hex parse, background/frame/connection
      color resolution, missing-variable pass-through, document
      action round-trip, backwards compat without `document_actions`,
      theme-demo loader + scene build + full round-trip
- [x] All 248 tests green (`./test.sh`)

**Key files**:
- `lib/baumhard/src/mindmap/model.rs` — `Canvas` fields
- `lib/baumhard/src/util/color.rs` — `resolve_var`, `hex_to_rgba_safe`
- `lib/baumhard/src/mindmap/custom_mutation.rs` — `DocumentAction`,
  `document_actions` field, round-trip tests
- `lib/baumhard/src/mindmap/scene_builder.rs` — color resolution at
  all render-build sites
- `lib/baumhard/src/mindmap/tree_builder.rs` — text-run color
  resolution through the Baumhard tree path
- `lib/baumhard/src/mindmap/loader.rs` — theme-demo round-trip tests
- `src/application/document.rs` — `Canvas` import, `DocumentAction`
  import, `UndoAction::CanvasSnapshot`, `apply_document_actions`
- `src/application/app.rs` — `handle_click` trigger dispatch,
  `cursor_is_hand` tracking, `CursorIcon::Pointer` transitions
- `maps/theme_demo.mindmap.json` — the sample map

**Verify**: `cargo run --release -- maps/theme_demo.mindmap.json`,
click each of the three button nodes, watch the background / frames /
edges / text colors swap instantly. Ctrl+Z restores the previous
theme. `./test.sh` is green.

### Animated mutations (planned, not yet built)

**What**: Wrap the existing mutation flow with timing — not a new
mutation kind, just an optional timing envelope on `CustomMutation`
(`duration_ms`, `delay_ms`, `easing`, `then: Reverse { hold_ms } |
Chain { id } | Loop`). At animation start, snapshot the from and to
states of affected nodes; each frame write a blended snapshot back
into `self.mindmap.nodes` so the existing `rebuild_all` path just
reads the in-progress state and repaints. Position, size, and all
color fields (including per-text-run colors) lerp; structural changes
snap at the boundary. While any animation is active the event loop
flips from `ControlFlow::Poll` to `WaitUntil(now + 16ms)` and ticks
the controller in `AboutToWait`. Phase-1's instant theme swap becomes
a smooth transition, and the same machinery drives pulses, reveals,
and any other declarative effect.

- [ ] `AnimationTiming { duration_ms, delay_ms, easing, then: Option<Followup> }`
      on `CustomMutation`, serde-default so old maps still load
- [ ] `Easing` enum (Linear / EaseIn / EaseOut / EaseInOut)
- [ ] `Followup::{Reverse, Chain, Loop}`
- [ ] Small purpose-built `lib/baumhard/src/mindmap/animation.rs`
      with the lerp + tick logic. The dormant
      `lib/baumhard/src/core/animation.rs` skeleton is left alone —
      it's generic over `T: Mutable` and would cost more to adapt
      than to replace.
- [ ] `active_animations: Vec<AnimationInstance>` on `MindMapDocument`,
      carrying from/to snapshots, phase, and timing
- [ ] `start_animation`, `tick_animations`, `has_active_animations`
      methods on the document
- [ ] Conditional `ControlFlow::WaitUntil` in `AboutToWait` while
      animations are active; back to `Poll` when idle
- [ ] `handle_click` branches on `cm.timing.is_some()` to start an
      animation instead of applying instantly
- [ ] Ctrl+Z during an animation fast-forwards to the end state then
      pops the undo entry — single predictable semantics
- [ ] Re-triggering the same (mutation_id, node_id) mid-flight is a
      silent no-op in v1
- [ ] `then: Reverse` forces Toggle semantics regardless of declared
      behavior (a reversed animation has no net persistent effect);
      log a warning at registry-build time if it was declared
      Persistent
- [ ] Non-interpolable mutations (`ModelCommand`, etc.) fire at the
      animation boundary, not per-frame
- [ ] Tests for timing round-trip, linear midpoint blend, completion,
      reverse followup, and post-completion undo

### Keyboard map navigation with feel (planned, not yet built)

**What**: Pan and zoom from the keyboard should feel like steering a
responsive spaceship. New action variants in the keybind layer so
WASD, arrow keys, and Colemak's WARS are interchangeable, held-key
tracking, and a per-axis velocity integrated each frame with a short
ease-in on press and ease-out on release. Zoom follows the same
curve against a log-scale factor so each tick multiplies rather than
adds.

- [ ] `Action::{PanUp, PanDown, PanLeft, PanRight, ZoomIn, ZoomOut}`
- [ ] Multiple default bindings per action (`["w", "ArrowUp"]` etc.)
- [ ] Held-key tracking: handle `ElementState::Released` alongside
      Pressed, maintain a `NavigationState` of directions currently
      down
- [ ] Per-axis velocity integrated in `AboutToWait`, fed into
      `RenderDecree::CameraPan` / `CameraZoom` (both already exist)
- [ ] Acceleration curve: baseline speed on press, ramp to cap over
      ~300ms, ease-out on release
- [ ] Log-scale zoom so each step multiplies the factor
- [ ] Native first — the WASM input path is a known gap and mustn't
      regress

### Connection & border render cost (in progress)

**Context**: The stutter users see during fast drags is not a generic
"mutation lag" problem. It is an efficiency gap in how connections
and borders are rebuilt on the renderer side — every drag frame
`Vec::clear()`s and rebuilds every connection and every border, with
a fresh cosmic-text buffer per glyph sample. The cost scales with
connection *length* (via glyph sample count) and with total map
size, not with what the user is actually touching. Neither
connections nor borders currently have any throttling, culling, or
incremental rebuild.

**Governing invariant** (set down at the top of §5 of the session
plan and reiterated by the user): responsiveness is never traded
for visual fidelity. The moment per-frame work threatens the screen
refresh budget, we sacrifice **frequency of mutation** — we apply
tree mutations (and the rebuilds they drive) less often — so that
input stays snappy. Input accumulation is sacred; mutation
application is the valve we turn. Everything below is either "ways
to never hit the wall" or "how to degrade gracefully when we do."

Five-part fix under that invariant:

- [x] **(E) Mutation-frequency throttle**. Moving average on measured
      per-frame work duration in `AboutToWait`; when it crosses the
      refresh budget, the drain rate of `pending_delta` drops to one
      in every N ticks, N ramping with how far over budget we are,
      decaying back to 1 when healthy. Between drains, mouse motion
      still accumulates — nothing is lost. Always-on, self-tuning,
      configuration-free. This is the invariant in code form.
      Implemented in `src/application/frame_throttle.rs` as
      `MutationFrequencyThrottle` with an 8-frame moving window, a
      default 14ms budget (60 Hz minus 2.7ms safety margin — correct
      value on higher-refresh monitors is an open tuning question),
      a 30% hysteresis band between raise and lower thresholds to
      prevent oscillation, and a cap of N = 8 (worst-case 133ms lag
      at 60 Hz before the other Phase 4 fixes kick in). Wired into
      `app.rs` `AboutToWait` drag branch, reset on drag release so
      fresh drags start at N = 1. 13 unit tests: healthy-load
      no-op, sustained-over-budget raise, cap-at-MAX_N,
      load-drop-decay, hysteresis-prevents-oscillation, throttled
      frames skip work, moving-average arithmetic, window eviction,
      reset returns to fresh state, drain cadence exactly matches
      N, default budget sanity, zero-frame empty-window.
- [x] **(A) Viewport culling on connection glyph samples**. Compute
      the canvas-space viewport in `rebuild_connection_buffers` and
      skip glyph positions that land off-screen. A few lines; kills
      the long-connection pathological case because the bulk of the
      glyphs on an "unreasonably long" edge live outside the visible
      rect while you're dragging an endpoint. Implemented by
      computing `vp_min`/`vp_max` from `camera.screen_to_canvas` on
      the surface corners once per call, then checking each glyph
      position (caps included) against a `font_size`-padded rect
      before calling `create_border_buffer`. The pad margin avoids
      visible popping at viewport edges during pan. The existing
      downstream cull in `render()` was only saving rasterization
      of already-shaped buffers; moving the cull upstream skips the
      cosmic-text shaping entirely, which is where the cost lives.
      The predicate is extracted to a free `glyph_position_in_viewport`
      function so it's testable without a `Renderer`. 7 new unit
      tests cover: center-of-viewport acceptance, edge inclusivity,
      far-off-screen rejection, margin expansion, just-past-margin
      rejection, non-origin viewport handling, and a scenario test
      that simulates a 20,000 canvas-unit connection with a 400x400
      viewport and confirms the cull drops ~1,334 glyph samples to
      ~28 (a 48× shaping-work reduction for the user's exact
      long-connection stutter case).
- [x] **(B) Keyed incremental rebuild**. Two caches, both targeted,
      neither general. The document-side `SceneConnectionCache` in
      `lib/baumhard/src/mindmap/scene_cache.rs` stores the *pre-clip*
      sampled positions + glyph config per edge, keyed by
      `EdgeKey(from_id, to_id, edge_type)`, with a `by_node:
      HashMap<String, Vec<EdgeKey>>` reverse index so a moved node
      dirties exactly its touching edges in O(k_N) rather than an
      O(E) walk. `build_scene_with_cache` consults the cache: if
      neither endpoint is in the drag `offsets` map and the entry is
      present, it clones the cached pre-clip samples and re-runs the
      cheap `point_inside_any_node` clip filter against the current
      frame's `node_aabbs` — a stable long cross-link therefore still
      clips correctly around a moved-but-unrelated third node, which
      is the governing-invariant correctness property. Dirty or
      missing edges take the slow path (`sample_path` + clip) and
      write the fresh entry back. Pre-clip samples are stored
      separately from the post-clip `glyph_positions` so selection
      changes and frame-specific clipping never invalidate the cache;
      the selected-edge color override is applied at read time.
      `ConnectionElement` gained a `pub edge_key: EdgeKey` so the
      renderer can key its buffer map without re-deriving it.
      Renderer-side: `border_buffers` and `connection_buffers` became
      keyed `FxHashMap<K, Vec<MindMapTextBuffer>>`. New
      `rebuild_border_buffers_keyed` / `rebuild_connection_buffers_keyed`
      methods accept an optional `dirty` set: clean entries with
      matching cap visibility + glyph count get only their `pos`
      patched in place (zero cosmic-text shaping), dirty entries are
      re-shaped. Sweep pass evicts unseen keys at the end. Camera
      pan/zoom/resize clears only the renderer-side `connection_buffers`
      (viewport cull output changes) while the document-side
      `SceneConnectionCache` holds camera-independent geometry and is
      NOT cleared — geometry stays cached across pans. Each drag
      starts with a freshly-cleared `SceneConnectionCache`; the first
      drag frame forces a full renderer rebuild (passes `None` for
      dirty sets) to avoid cap-visibility / glyph-count mismatches
      against pre-drag shaped buffers, then frames 2+ run the
      incremental path. 18 new unit tests split across
      `scene_cache::tests` (7: insert/get round-trip, by-node index,
      multiple-edges-per-node, invalidate, clear, retain-keys
      eviction, reinsert-idempotency) and `scene_builder::tests::test_cache_*`
      (11: populate-on-first-build, cache-hit-preserves-identity,
      invalidate-on-endpoint-offset, preserve-unrelated-edge-under-drag,
      clip-reruns-against-fresh-aabbs — the correctness of the
      governing invariant for cached edges, evict-deleted-edges,
      selection-change-does-not-invalidate, edge-key-always-populated,
      second-cache-hit-identical-output, empty-after-new,
      fold-hidden-edge-not-cached). 298 tests passing (up from 280).
      Eliminates the ~1,700 bezier evaluations/frame/long-edge that
      Phase A left untouched — the upstream geometry cost. Together
      with Phase A, drag cost on the user's long-connection scenario
      now scales with what moved, not with what exists.
- [ ] **(C) Shape-once-reuse**. On top of (B): cosmic-text shaping
      is the expensive step, positioning is cheap. Keep shaped
      buffers alive across frames and only update their positions
      when the content is unchanged — the common case during drag.
      Node text buffers (`rebuild_buffers_from_tree` in `renderer.rs`)
      still re-shape every visible node every drag frame; that's the
      next fish once Phase B proves itself. Phase B already
      implements a form of shape-once-reuse for connection and
      border buffers — Phase C extends it to the tree-based node
      text path.
- [ ] **(D) Sample decimation during motion**. Double the effective
      spacing passed to `sample_path` while a drag is active; halves
      connection glyph counts invisibly under motion. Held in
      reserve; may prove unnecessary once (A)+(B)+(C) land.

**Other bottlenecks surfaced during Phase B investigation** (not
yet addressed; call-outs for follow-up sessions):

- `point_inside_any_node` is O(N) per point. After Phase B the clip
  filter becomes the dominant per-edge cost during drag because the
  sampler is now skipped. A 1D sort on x (or a coarse grid index)
  would drop it to O(log N) / O(1).
- `doc.mindmap.all_descendants(nid)` is recomputed inside the drag
  drain block every frame. Cheap for small subtrees, but O(subtree)
  per frame when dragging a large subtree. Cache once in
  `DragState::MovingNode` at drag start.
- `resolve_var(...).to_string()` in both `scene_builder.rs` and
  `tree_builder.rs` allocates per-element per-frame. Resolve the
  canvas-level palette once per scene build and pass a
  `&HashMap<&str, &str>` down.
- `MutationFrequencyThrottle` uses a hardcoded 14 ms budget. On a
  144 Hz monitor this is loose; the throttle never engages even when
  it could help. A monitor-refresh-aware budget is a one-liner once
  the winit frame pacing API exposes the refresh rate.

**Ordering**: E → A → B → C → (D if needed). (E) lands first as the
safety net; subsequent fixes reduce how often it has to engage.
Borders ride on (B) without needing their own culling pass —
they're always colocated with their node.

### Stress-test map generator

**What**: A `[[bin]]` target in the `baumhard` crate that emits
`.mindmap.json` files of configurable size and topology, seedable
for reproducibility. Doubles as the smoke-test rig for Phase 4 —
each of the five sub-fixes above wants a measurable before/after,
and the stress generator is how we get one. Not a benchmark
harness, just a file writer.

- [x] New `[[bin]]` target `generate_stress_map` in
      `lib/baumhard/src/bin/generate_stress_map.rs`. Auto-discovered
      by cargo; no `[[bin]]` entry needed in `Cargo.toml` because
      baumhard is a library crate with a `src/bin` directory.
- [x] CLI flags: `--topology`, `--nodes`, `--depth`, `--branching`,
      `--cross-links`, `--long-edges`, `--seed`, `--output`, `--help`.
      Manual argument parsing (no clap) to match the
      file-writer-only ethos.
- [x] Three topologies: `balanced` (complete tree of given depth and
      branching, centred grid layout), `skewed` (comb — a long
      diagonal spine with one leaf per interior node), `star` (one
      root with N-1 children laid out on a circle around it).
      Exercises deep hierarchies, deep paths, and massive sibling
      lists respectively.
- [x] `--long-edges K` knob: adds K cross-link edges between the
      most-distant node pairs in the layout. The key long-connection
      perf-test lever — these are exactly the edges whose glyph
      sampling count blows the frame budget. A balanced tree at
      depth 4, branching 3, with `--long-edges 2` yields a 25,600
      canvas-unit cross-link on the default layout, which at typical
      font sizes produces ~1,700 glyph samples per frame during drag.
- [x] `--cross-links K` knob: adds K random cross-link edges,
      avoiding self-links and duplicates. Simulates messy real-world
      maps.
- [x] Uses `baumhard::mindmap::model` serde types directly — no
      hand-rolled JSON. Serialises via `serde_json::to_string_pretty`.
- [x] Deterministic output per `--seed` (default `0xBAADF00D`).
      Accepts hex (`0xDEADBEEF`) or decimal seeds.
- [x] 12 new tests cover: topology parsing, balanced tree node-count
      formula, balanced single-root invariant, skewed node count,
      skewed edge shape, star root/children invariants, single-node
      star, cross-link no-self-link guarantee, longest-pair selection,
      full serde round-trip through the generated `MindMap`,
      seed determinism.

**Key files**:
- `lib/baumhard/src/bin/generate_stress_map.rs` — the binary

**Verify**:
```shell
cargo run -p baumhard --bin generate_stress_map -- \
    --topology balanced --depth 4 --branching 3 --long-edges 2 \
    --output maps/stress.mindmap.json
cargo run -- maps/stress.mindmap.json
```
Loads into the app, renders correctly, and the long cross-link is
visually present and stretches far off-screen. `./test.sh` green
(260 tests, 12 new).

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
