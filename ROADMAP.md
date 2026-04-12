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
- **CLI console replaces the command palette** — the `/`-triggered UI is now a shell-style CLI rather than a pick-list. `src/application/console/` houses a command registry (`COMMANDS: &[Command]`, one file per verb under `console/commands/`), a shell-style parser (whitespace + `"quoted strings"` + `--flag=value` / `--flag value`), a contextual completion engine (Tab cycles; command names at position 0, enum values at known subcommand positions, node ids for `select`, mutation ids for `mutate run`), and a grapheme-safe line editor routed through `baumhard::util::grapheme_chad`. The former palette's ~54 actions collapse into 12 commands — `anchor`, `body`, `cap`, `color`, `connection`, `edge`, `font`, `label`, `portal`, `spacing`, plus the new `help`, `select`, `mutate`, `alias`. `help` filters the visible command list by applicability to the current selection by default (`help --all` lists everything). `select <none|node <id>>` navigates selection from the console. `mutate list` prints the merged registry with source classification (user / map / inline); `mutate run <id> [node_id]` applies on the Single selection via the existing `apply_custom_mutation` undo path. `mutate bind <key> <id>` persists a key-combo → mutation mapping to `$XDG_CONFIG_HOME/mandala/console_bindings.json` (a dedicated overlay file so the user's `keybinds.json` isn't trampled); `mutate unbind <key>` removes it. `alias <name> <expansion...> [--save]` defines a first-token shortcut — session-only without `--save`, persisted to the user-mutations file's `aliases` field with it. User-defined mutations load at startup from `$XDG_CONFIG_HOME/mandala/mutations.json` at the lowest registry precedence (user < map < inline). Command history persists to `$XDG_STATE_HOME/mandala/history` with a 500-line cap. The overlay is bottom-anchored (prompt row, completion popup, scrollback region above), rendered via the same second-TextRenderer the palette used, with the border, font family, and font size all configurable from `keybinds.json` (`console_border` default `"#"`, `console_font` empty = cosmic-text fallback, `console_font_size` default 16.0). The border unit string is tiled horizontally on top/bottom and its characters stacked vertically (rotated) on the sides. Trigger is a first-class keybind (`open_console`, default `["/"]`); pressing it while the console is open toggles it closed. Suppressed while any other keyboard-capturing modal (`LabelEditState`, `ColorPickerState`, `TextEditState`) is active. Backward-compat is locked in by a `BACKCOMPAT_INVOCATIONS` table that maps every former palette action id to a console invocation and is exercised by a table-driven test.
- **Connection style editing via console** — Session 6D landed the style setters behind palette actions; the console replacement kept the setters and surfaced them as commands. Surface: `body <dot|dash|double|wave|chain>`, `cap <from|to> <arrow|circle|diamond|none>`, `color <pick|accent|edge|fg|reset>` (theme-var-aware), `font <smaller|larger|reset>` (±2pt step, clamp-aware), `spacing <tight|normal|wide>`, `edge type <cross_link|parent_child>` (duplicate-guarded), `connection reset-style` / `connection reset-straight`. Every mutation goes through an `ensure_glyph_connection` fork helper that materializes a concrete per-edge copy from the effective resolved config on first edit — forked copies retain their values when the canvas default later changes, and undo restores the pre-fork None cleanly because the `before` snapshot is taken before the fork. Uses the existing `UndoAction::EditEdge` variant — no new undo machinery.
- **Connection labels** — Session 6D. `MindEdge.label` (which had existed in the data model since the pre-roadmap era but was never drawn) now renders as a `ConnectionLabelElement` along the edge's path at a parameter-space position `edge.label_position_t` (new field, defaulting to 0.5 at the midpoint). Labels render through the flat scene pipeline as individual cosmic-text buffers, slightly larger than the connection body glyphs for readability (1.1× font size). Color resolves through the same theme-variable path as the connection body, so a `var(--accent)` color auto-restyles on theme swap. Labels are rebuilt every scene build (no cache — ≤ 1 per edge, trivially cheap) so they track live drags of either endpoint.
- **Inline label edit modal** — Session 6D. Click the rendered label on a selected edge to enter `LabelEditState::Open { edge_ref, buffer, original }`. Like the command palette, the modal steals all keyboard input: Escape discards (restoring the pre-edit value), Enter commits via `document.set_edge_label` (which pushes an `EditEdge` undo entry), Backspace pops, character keys append. A static `▌` caret appears after the edited text via `Renderer::label_edit_override: Option<(EdgeKey, String)>`, which `rebuild_connection_label_buffers` consults on every keystroke. Cursor navigation is deferred — the buffer is append-only for Session 6D. The palette "Edit connection label" action is the fallback entry point for edges without committed labels (whose text glyphs aren't hit-testable yet).
- **Portal pairs** — Session 6E. A new `MindMap.portals: Vec<PortalPair>` collection (parallel to `edges`) holds matching glyph markers that visually link two distant nodes without drawing a connection line across the canvas — a lightweight alternative to cross-link edges for far-apart endpoints. Each pair emits *two* `PortalElement`s in the scene (one floating above the top-right corner of each endpoint), both sharing the same glyph, color, and auto-assigned label. Labels progress through column letters A..Z..AA..AB.. with gap-reuse (`MindMap::next_portal_label` picks the lowest unused letter so deleting "B" and creating a new portal reuses "B"). Default glyphs rotate through `PORTAL_GLYPH_PRESETS = ["◈","◆","⬡","⬢","◉","❖","✦","✧"]` by `portals.len() % 8` so successive creations look distinct. Portals render through the flat scene pipeline (no tree involvement); the `Renderer` keys a `portal_buffers: FxHashMap<(PortalRefKey, String), MindMapTextBuffer>` plus parallel `portal_hitboxes` for click detection. `Renderer::hit_test_portal` is consulted in `handle_click` between the node and edge hit tests so clicks on a marker floating above a node corner don't fall through. Selection via `SelectionState::Portal(PortalRef)` (mutually exclusive with node/edge selection); `selected_portal` is threaded through the scene builder parallel to `selected_edge` so both markers of a selected pair render in the cyan highlight color. Delete removes the pair via `apply_delete_portal`; Ctrl+Z restores it via three new undo variants (`CreatePortal` / `DeletePortal` / `EditPortal`). The command palette grew from 42 to 52 actions with 10 new 6E entries: "Create portal" (applicable when Multi selection has exactly 2 nodes), "Delete portal", 4 glyph presets (hexagon/diamond/star/circle), and 4 theme-var-aware colors (accent/edge/fg/reset). Portal colors resolve through `resolve_var` so `var(--accent)` auto-restyles on theme swap. Rename-portal-label and free-form glyph input are deferred (would need the inline label edit modal infrastructure).
- **Glyph-wheel color picker** — A custom mandala-shaped color picker built entirely from positioned cosmic-text glyphs, opened from the command palette via "Pick edge color…" / "Pick portal color…" entries (or any future opener that wants a magical color UI). Layout: a 24-glyph hue ring (`●` filled circles colored at `hsv_to_rgb(i*15°, 1, 1)`, with the current hue slot swapping to `◉` fisheye), a crosshair sat/value selector formed by two perpendicular `■`/`◆` glyph bars (11 cells each — horizontal = saturation at the current hue+val, vertical = value at the current hue+sat), a central `✦` preview glyph at 2× font size showing the live HSV pick, a small hex readout below it, and a row of theme-variable quick-pick chips (`--accent`, `--bg`, `--fg`, `--edge`, reset). Mouse hover live-previews on the actual edge/portal behind the picker via the new non-undoing `MindMapDocument::preview_edge_color` / `preview_portal_color` helpers; click commits at the hit position; click-outside or `Esc` cancels and restores the captured pre-picker snapshot. Keyboard fallback: `h`/`H` ±15° hue, `s`/`S` ±0.1 sat, `v`/`V` ±0.1 val, `Tab` cycles theme chips, `Enter` commits, `Esc` cancels — same modal-keyboard pattern as the palette and label-edit modals. Live preview avoids the new-renderer-override-field complication entirely by mirroring the `apply_edge_handle_drag` snapshot pattern at `app.rs:505-537` — capture the pre-picker `MindEdge`/`PortalPair` clone at open time, mutate the model in place during hover (no undo push), and on commit push exactly one `UndoAction::EditEdge` / `EditPortal` carrying the captured `before` snapshot. Cancel restores the snapshot in place without touching the undo stack. v1 ships two `ColorTarget` variants (`Edge(EdgeRef)` / `Portal(PortalRef)`); the enum is designed to be extended to NodeStyle targets in a follow-up session by adding match arms and node setters. The renderer reuses the existing `palette_text_renderer` glyphon pass and the existing palette backdrop rect batch — picker and palette are mutually exclusive modal overlays so they share infrastructure cleanly. New `lib/baumhard/src/util/color.rs` helpers `hsv_to_rgb`, `rgb_to_hsv`, `hsv_to_hex`, `hex_to_hsv_safe` are the canonical color-math home, with round-trip tests against the six primary hues plus black/white/mid-gray.
- **Keyboard node editing + deletion** — Session 7A follow-up. **Enter** on a selected single node opens the inline text editor on its existing text (cursor at end); **Backspace** opens the editor with an empty buffer ("clean slate" retype — commit replaces the text wholesale); **Delete** removes the selected node, orphaning its immediate children (they become roots) and stripping every edge that touched the deleted node. Both edit actions are new `Action::EditSelection` / `Action::EditSelectionClean` variants in `keybinds.rs` with configurable default bindings (`Enter` / `Backspace`); node deletion is wired into the existing `Action::DeleteSelection` match by extending the local `DelKind` enum with `Node(String)` / `Nodes(Vec<String>)` arms. The model-side writer is `MindMapDocument::delete_node` — it snapshots the node, orphans children with fresh root indices (mirroring `apply_create_orphan_node`'s indexing), removes every incident edge in ascending index order, and returns an `UndoAction::DeleteNode { node, removed_edges, orphaned_children }` payload the caller pushes onto the undo stack. The undo arm re-inserts the node, re-inserts every edge at its original index (ascending order + LIFO undo make this safe without re-ordering bookkeeping), and restores each orphaned child's pre-delete `parent_id` + `index`. **Regions bug fix**: Session 7A's `apply_text_edit_to_tree` sent a `DeltaGlyphArea { Text: Assign }` without touching `ColorFontRegions` — on a pre-existing multi-run node with regions referencing the original character ranges, the renderer's region-filtered span builder at `renderer.rs:1500-1520` silently dropped the caret glyph (appended at `text.len()`) and any chars typed beyond the original length, making it appear as if double-click did nothing. Fixed by also sending a `ColorFontRegions` field in the delta with a single region spanning `[0, display_char_count)` that inherits the color of the first pre-edit region; `apply_operation` assigns the regions wholesale (area.rs:261). Regression test `test_text_edit_replaces_stale_regions_to_cover_caret` guards against a recurrence.
- **Inline node text editor** — Session 7A. Double-click any node to open an inline multi-line editor; double-click empty canvas to create a new orphan node and open the editor on it. Keystrokes flow through Baumhard's existing `Mutation::AreaDelta` vocabulary — each frame, the app builds a display string (`buffer` with a `▌` caret glyph inserted at `cursor_char_pos`) and wraps it in `DeltaGlyphArea::new(vec![GlyphAreaField::Text(display_text), GlyphAreaField::Operation(ApplyOperation::Assign)])`, then applies it to the target node's live `GlyphArea` via `glyph_area_mut()` + `mutation.apply_to(area)` + `renderer.rebuild_buffers_from_tree(&tree.tree)`. The model (`MindNode.text` / `text_runs`) is **untouched during typing** — transient state lives only on the tree. Commit is via click-outside-the-edited-node; Esc cancels (a plain `rebuild_all` rebuilds the tree from the unchanged model, discarding the transient edits wholesale). `TextEditState::Open { node_id, buffer, cursor_char_pos, original_text, original_runs }` lives on the app layer, mutually exclusive with palette/color picker/label-edit via the same early-return keyboard-steal pattern at `app.rs` ~L1166. `document.set_node_text(node_id, new_text)` is the commit-side model writer — mirrors `set_edge_label` at `document.rs:907`, pushes one `UndoAction::EditNodeText { node_id, before_text, before_runs }` if the text actually changed, and **collapses** `text_runs` to a single run inheriting the first original run's formatting (font, size_pt, color, bold, italic, underline) — authored multi-run nodes lose per-span styling on first edit; rich-text-preserving edits are the entire reason Session 7B's TextRun splitting is deferred. **Multi-line by design**: Enter inserts `'\n'`, Tab inserts `'\t'` — this is a paragraph editor, not an outliner. Cursor navigation is line-aware via simple `\n`-scanning helpers (`cursor_to_line_start`, `cursor_to_line_end`, `move_cursor_up_line`, `move_cursor_down_line`); Home/End jump to line bounds; Left/Right walk by char; Backspace/Delete delete before/after cursor. Cursor positions are char indices (not byte offsets), converted via `buffer.char_indices().nth(n)` at insert/delete time — grapheme-cluster awareness deferred. Double-click detection fires on second `Pressed` within 400ms and 16px² of the first, via the pure `is_double_click` helper that compares `(time, screen_pos, hit)`. Click-outside commits at the `DragState::Pending → Released` branch in `app.rs` ~L700: if the release canvas position falls outside the edited node's AABB, `close_text_edit(commit=true)` fires; inside the AABB, the release is swallowed (keep editing, no selection change). No new keybinds — the in-modal keys (Esc, arrows, Home/End, Enter, Tab, printable chars) follow the hardcoded precedent set by the palette and label-edit modals; the user's "all keybinds modifiable" constraint is satisfied vacuously since no dispatch outside modal mode is affected. Text selection (shift+arrow), rich text toggles (Ctrl+B/I), clipboard (Ctrl+C/X/V), click-to-position-cursor, IME composition, soft-wrap-aware cursor, and auto-grow-node-on-overflow are all explicitly deferred to Session 7B.
- **Sacred-script color picker glyphs** — Visual refactor of the glyph-wheel picker from geometric dingbats (`●` `■` `◆` `✦` `◯`) to sacred scripts, matching the mandala aesthetic. The 24-slot hue ring is now three 8-glyph arcs clockwise from 12 o'clock: Devanagari consonants (क ख ग घ च ज ट ड), Hebrew alefbet (א ב ג ד ה ו ז ח), and Tibetan column heads (ཀ ཁ ག ང ཅ ཏ པ མ), rendered at `font_size * HUE_RING_FONT_SCALE` (1.5×) so the ring reads as the dominant visual element. The crosshair is now a four-armed typographic compass — `SAT_CELL_COUNT` / `VAL_CELL_COUNT` doubled `11 → 21` so each bar has two 10-cell arms plus a shared wheel-center cell (skipped during rendering so the central ॐ shows through cleanly). Top arm is Devanagari independent vowels (अ आ इ ई उ ऊ ऋ ए ऐ ओ, brightest → middle value), bottom arm is Hebrew letters 9-18 (ט י כ ל מ נ ס ע פ צ, middle → darkest), left arm is Tibetan consonants not in the ring (ཉ ཐ ད ན ཕ བ ཙ ཞ ར ས, desaturated → middle), right arm is Egyptian Hieroglyph narrow uniliterals from Gardiner's alphabet (𓇋 𓅱 𓃀 𓊪 𓆑 𓈖 𓂋 𓉔 𓋴 𓏏, middle → saturated). The center preview is ॐ (U+0950) at 2× size, replacing `✦`. The selected-cell highlight swaps from a `■`→`◆` glyph swap to a brighten-toward-white color tint so per-cell script identity is preserved. The hex readout is now hidden by default — the old `(center.x - char_width*4, center.y + preview_size*0.7)` position collided with the lower val-bar cells — and appears only when the cursor is inside the backdrop or a theme chip is focused, anchored below the chip row and horizontally centered on the wheel. Cell spacing is runtime-measured via cosmic-text's shaping pass (new `measure_max_glyph_advance` helper in `renderer.rs`) at picker open, with the widest advance across all 40 crosshair glyphs used as the unit so all four arms stay symmetric regardless of per-script width variation. Four Noto font files (Noto Sans Devanagari, Noto Sans Hebrew, Noto Serif Tibetan, Noto Sans Egyptian Hieroglyphs) were added to `lib/baumhard/src/font/fonts/` and are auto-discovered by `build.rs` — the enum grew from 48 to 52 variants with no manual wiring. cosmic-text's script-based font fallback picks up the right Noto font per script automatically (verified the same way the palette's existing ॐ / אל border works), and embedding the fonts into the binary ensures WASM parity with native. New layout fields `cell_advance`, `ring_font_size`, `hex_pos: Option<(f32, f32)>` keep the renderer in sync with the pure layout fn. New tests: `hue_ring_slots_do_not_overlap_at_new_font_scale`, `hex_pos_is_some_iff_hex_visible`, `hex_pos_horizontally_centered_on_wheel_center`, `crosshair_arms_render_exactly_10_cells_each`, `crosshair_arms_emit_symmetric_cell_advance`, `hue_ring_glyphs_are_grouped_by_script`.
- **WASM input parity (most)** — Session 7B. The Session 7A text editor, which originally shipped behind `#[cfg(not(target_arch = "wasm32"))]`, now works in the browser. WASM `run` grew a real input layer: canvas `tabindex` + `focus()` + a `mousedown` refocus listener + a `keydown` `preventDefault` listener (gated on editor-open) so browsers don't swallow Tab/Enter/Backspace/arrows; a cross-platform `now_ms()` replacing `Instant` in `LastClick` / `is_double_click`; `Rc<RefCell<Option<_>>>` shared state (one for `Renderer`, one for the input struct) bridging the rAF loop and the winit event loop; full keyboard dispatch routing through `ResolvedKeybinds::action_for` with modifier tracking (`ModifiersChanged`) and handlers for `Undo`, `EditSelection` / `EditSelectionClean`, `CreateOrphanNode`, `OrphanSelection`, `DeleteSelection`, and `CancelMode`; mouse-wheel zoom via `RenderDecree::CameraZoom`. All Session 7A pure text-edit helpers de-gated and now reachable from both targets. Still deferred on WASM: drag-to-move nodes, drag-to-pan, `EnterReparentMode` / `EnterConnectMode` (need `AppMode` de-gated), and the palette / label-edit / color-picker modals (their state types and rebuild paths are still native-gated).
- **Multi-target** — native + WASM builds
- **667 tests passing** — run via `./test.sh`; see `TEST_CONVENTIONS.md` for the testing philosophy and `./test.sh --coverage` for `cargo-llvm-cov` reports

### What needs work
- **No rich-text-preserving text editing** — Session 7A lands the core inline multi-line editor; text selection, rich text toggles (Ctrl+B/I), clipboard, click-to-position-cursor, IME, and soft-wrap-aware cursor are all deferred to Session 7B. Authored multi-run nodes collapse to a single run on first edit.
- **Borders not in tree** — borders render via flat pipeline, not as GlyphModel children in the Baumhard tree
- **No save/persistence** — document dirty flag exists but no serialization
- **Hover and key trigger dispatch** — `OnHover` and `OnKey` triggers are in the data model but the event loop only dispatches `OnClick` so far
- **Global custom mutations** — `~/.mandala/custom_mutations.json` loading is not yet wired
- **WASM input** — partial. Session 7B closed most of the gap: click to select, double-click to open/create the text editor, full text-edit keyboard flow, click-outside-commit, mouse-wheel zoom, and keyboard hotkey dispatch for `Undo`, `EditSelection`, `EditSelectionClean`, `CreateOrphanNode`, `OrphanSelection`, `DeleteSelection`, `CancelMode`. Still deferred on WASM: drag-to-move nodes, drag-to-pan, `EnterReparentMode` / `EnterConnectMode` (need `AppMode` de-gated), the palette / label-edit / color-picker modals (their state types and rebuild paths are all native-gated today), and hover/key trigger dispatch.
- **Animated mutations** — mutations currently apply instantly; the timing/interpolation layer planned in Session 10B is not yet built
- **Keyboard map navigation** — no keyboard pan/zoom; mouse-driven only
- **Fast-pan stutter** — model mutation work can fall behind input during very fast drags

### Key Files Reference
| File | Role |
|------|------|
| `src/application/app.rs` | Event loop, window, DragState machine, input handling, tree+scene pipeline wiring. Color picker: `open_color_picker`, `commit_color_picker`, `cancel_color_picker`, `apply_picker_preview`, `apply_picker_chip`, `handle_color_picker_key`, `handle_color_picker_mouse_move`, `handle_color_picker_click`, `rebuild_color_picker_overlay` |
| `src/application/color_picker.rs` | Glyph-wheel color picker module: `ColorTarget` enum (Edge / Portal), `ColorPickerState`, `ColorPickerSnapshot`, `ColorPickerOverlayGeometry`, `ColorPickerLayout`, pure-fn `compute_color_picker_layout`, `hit_test_picker`, `THEME_CHIPS`, `HUE_SLOT_COUNT = 24` / `SAT_CELL_COUNT = 11` / `VAL_CELL_COUNT = 11` |
| `src/application/renderer.rs` | GPU pipeline: tree-based node rendering + flat border/connection rendering. Color picker overlay via `rebuild_color_picker_overlay_buffers` (mandala layout: 24-glyph hue ring, crosshair sat/value bars, center preview ✦, theme chips, hint footer); shares `palette_text_renderer` and the palette backdrop rect batch since the two modal overlays are mutually exclusive |
| `src/application/document.rs` | Owns MindMap + SelectionState + UndoStack, provides `build_tree()`, `build_scene()`, `hit_test()`, `apply_selection_highlight()`, `apply_drag_delta()`, `apply_move_subtree/single()`, `undo()`, `apply_custom_mutation()`, `apply_document_actions()`, the Session 6D edge-mutation suite (`ensure_glyph_connection`, `set_edge_body_glyph`, `set_edge_cap_start/end`, `set_edge_color`, `set_edge_font_size_step`, `reset_edge_font_size`, `set_edge_spacing`, `set_edge_label`, `set_edge_label_position`, `set_edge_type`, `reset_edge_style_to_default`), the Session 6E portal-mutation suite (`PortalRef`, `apply_create_portal`, `apply_delete_portal`, `apply_edit_portal`, `set_portal_glyph`, `set_portal_color`), and the non-undoing color-picker preview helpers `preview_edge_color` / `preview_portal_color` |
| `src/application/common.rs` | RenderDecree, WindowMode, InputMode, timing |
| `src/application/frame_throttle.rs` | `MutationFrequencyThrottle` — the governing-invariant safety net for the drag path |
| `src/application/keybinds.rs` | Configurable key-to-Action mapping with layered loading |
| `lib/baumhard/src/mindmap/model.rs` | MindMap, MindNode, MindEdge, Canvas (incl. `theme_variables`, `theme_variants`), `all_descendants()`, Session 6E `PortalPair`, `MindMap.portals`, `next_portal_label`, `column_letter_label`, `PORTAL_GLYPH_PRESETS` |
| `lib/baumhard/src/mindmap/tree_builder.rs` | MindMap → Tree<GfxElement, GfxMutator> bridge, resolves text-run colors through theme variables |
| `lib/baumhard/src/mindmap/scene_builder.rs` | MindMap → flat RenderScene (connections, borders, background), resolves colors through theme variables. Phase B: `build_scene_with_cache` reuses cached per-edge sample geometry. Session 6E: `PortalElement` + `PortalRefKey`, `selected_portal` threaded alongside `selected_edge`, portal emission post-pass |
| `lib/baumhard/src/mindmap/scene_cache.rs` | `SceneConnectionCache`, `EdgeKey`, `CachedConnection` — Phase B per-edge pre-clip sample cache with `by_node` reverse index |
| `lib/baumhard/src/mindmap/loader.rs` | JSON loading |
| `lib/baumhard/src/mindmap/border.rs` | BorderGlyphSet, BorderStyle |
| `lib/baumhard/src/mindmap/connection.rs` | Path computation, Bezier curves, glyph sampling |
| `lib/baumhard/src/mindmap/custom_mutation.rs` | `CustomMutation`, `TargetScope`, `MutationBehavior`, `Trigger`, `TriggerBinding`, `DocumentAction`, `PlatformContext` |
| `lib/baumhard/src/util/color.rs` | Hex parsing, `Color` arithmetic, `resolve_var()`, `hex_to_rgba_safe()`, glyph-wheel HSV helpers `hsv_to_rgb` / `rgb_to_hsv` / `hsv_to_hex` / `hex_to_hsv_safe` |
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

**What**: Customize connection appearance and add/edit labels. Extends
the Session 6C palette with 31 new actions for glyph presets, colors,
font size, spacing, edge type, label editing, and reset-to-default.
Lands the first rendering path for `MindEdge.label` (which had existed
in the data model since the pre-roadmap era but was never drawn), plus
an inline click-to-edit modal that steals keyboard input the same way
the command palette does.

- [x] Change connection glyph: body (5 presets: dot/dash/double/wave/chain), cap_start (arrow/circle/diamond/clear), cap_end (arrow/circle/diamond/clear) via palette actions
- [x] Change connection color via theme-var-aware palette actions (accent/edge/fg/reset) — stores `var(--name)` references so edges restyle on theme swap
- [x] Change connection font size via palette (smaller/larger/reset, ±2pt steps, clamp-aware applicability). Font family deferred to a future session — Mandala has no font picker pattern yet.
- [x] Change glyph spacing via palette (tight/normal/wide presets)
- [x] Edit connection label: click the rendered label on the selected edge to open the inline editor, type, Enter commits, Esc discards. Palette action "Edit connection label" is the fallback entry point. Live caret preview via the Renderer's `label_edit_override`; keystrokes steal input the same way the palette does.
- [x] Position label along connection path via palette (start/middle/end = t 0.0/0.5/1.0). New `MindEdge.label_position_t: Option<f32>` field, clamped into `[0, 1]` by the setter. Default None = 0.5 (middle) at render time.
- [x] Change edge type via palette (Convert to cross-link / Convert to parent-child). `set_edge_type` refreshes the selection `EdgeRef` after the change so the edge stays selected under its new identity; refuses conversions that would produce a duplicate `(from_id, to_id, new_type)` triple.
- [x] Apply canvas default style via palette action "Reset connection style to default" — clears `edge.glyph_connection` back to None so the edge falls through to the canvas-level default.
- [x] **Bonus: command palette overlay polish** — Phase 0 of the session fixed two visible bugs in the Session 6C palette: (1) the backdrop color was a dark blue-grey (`#0F121A`), changed to pitch black; (2) the backdrop rectangle extended `char_width` past the cyan border on each side horizontally and `font_size - 2` past the bottom border vertically, now snapped flush to the border bounds via a new `PaletteFrameLayout` pure helper. 5 new regression tests guard both invariants.
- [x] 42 new tests (5 palette-layout + 8 model/connection + 5 scene-builder label + 3 palette id/count + 14 document mutation + 3 palette id resolution + 4 pre-existing test count), `./test.sh` green at 408 tests (+42 over the 366 baseline).

**Fork semantic for per-edge style overrides**: on the first style
edit of an edge whose `glyph_connection` is `None`, the document
helpers materialize a concrete per-edge copy from the effective
resolved config (canvas default, else hardcoded default), then mutate
the one field the user asked to change. The `before` snapshot for the
undo entry is taken *before* the fork, so Ctrl+Z on the first edit
cleanly restores the pre-fork None — subsequent canvas-default
changes won't retroactively affect forked edges, mirroring how CSS
"computed style" copies work. Implemented via
`MindMapDocument::ensure_glyph_connection` + 10 `set_edge_*` helpers
that all follow the `reset_edge_to_straight` template from Session
6C. Undo uses the existing `UndoAction::EditEdge { index, before }`
variant — no new undo machinery.

**Label rendering**: `ConnectionLabelElement` (new in
`scene_builder.rs`) is emitted as a separate post-pass over
`map.edges` — labels are ≤ 1 per edge and rebuilt every scene build,
so no incremental cache is warranted. Position is computed via
`connection::point_at_t(&path, edge.label_position_t.unwrap_or(0.5))`,
font size is `config.effective_font_size_pt(camera_zoom) * 1.1`
(slightly larger than the body glyphs for readability), color
resolves through `resolve_var` like every other color field. The
renderer's `rebuild_connection_label_buffers` builds one
`MindMapTextBuffer` per label and stores an AABB hitbox in
`connection_label_hitboxes` for inline click detection. During a
drag (node move, edge handle drag, camera pan) labels are rebuilt
every frame alongside the connection buffers so they track the live
path.

**Inline label edit modal**: new `LabelEditState` enum in `app.rs`
mirrors the `PaletteState` shape. Entered via:
(a) `Open { edge_ref, buffer, original }` from the left-click handler
when the cursor hits a selected edge's label via
`Renderer::hit_test_edge_label`, or
(b) from the palette via `PaletteEffects::open_label_edit`, which the
dispatcher drains after `action.execute` returns and hands to
`open_label_edit()`. Keyboard input is stolen the same way the
palette does: Escape discards, Enter commits (via
`document.set_edge_label`), Backspace pops, `Key::Character` appends.
Live preview on every keystroke via `Renderer::label_edit_override:
Option<(EdgeKey, String)>`, which `rebuild_connection_label_buffers`
consults to substitute the edited text + a static `▌` caret for the
scene element's text. Cursor navigation is deferred — the buffer is
append-only for Session 6D.

**Key files**:
- `lib/baumhard/src/mindmap/model.rs` — `MindEdge.label_position_t`, `GlyphConnectionConfig::resolved_for`
- `lib/baumhard/src/mindmap/connection.rs` — `point_at_t` public wrapper over `cubic_bezier_point` (now `pub(crate)`)
- `lib/baumhard/src/mindmap/scene_builder.rs` — `ConnectionLabelElement` struct, `RenderScene::connection_label_elements` field, label emission post-pass inside `build_scene_with_cache`
- `src/application/document.rs` — `ensure_glyph_connection` fork helper + 10 `set_edge_*` mutation methods (body/cap_start/cap_end/color/font_size_step/font_size_reset/spacing/label/label_position/type/reset_style). All push `UndoAction::EditEdge`.
- `src/application/palette.rs` — `PaletteEffects::open_label_edit` channel, 31 new `PaletteAction` entries, 9 new applicability predicates + helper accessors for self-hiding actions, 3 new registry tests
- `src/application/app.rs` — `LabelEditState` enum + state, `handle_label_edit_key` keyboard handler, `open_label_edit` / `close_label_edit` helpers, palette dispatcher drains `open_label_edit`, label-hit intercept in the click-release branch, `rebuild_connection_label_buffers` wiring across `rebuild_all` + `rebuild_all_with_mode` + drag frame paths
- `src/application/renderer.rs` — `PaletteFrameLayout` pure helper + `compute_palette_frame_layout` (Phase 0), pitch-black palette backdrop color (Phase 0), backdrop rect flush to border bounds (Phase 0), `connection_label_buffers` + `connection_label_hitboxes` + `label_edit_override` fields, `rebuild_connection_label_buffers`, `hit_test_edge_label`, label buffers chained into the main text-area render pass

**Verify**: `./test.sh` green (408 tests, +42 new). `cargo run --release -- maps/theme_demo.mindmap.json`:

1. Press `/`, confirm the palette background is pitch black and the cyan border sits flush on all four sides (Phase 0).
2. Select a connection (cyan highlight).
3. `/` → `"dash"` Enter — body becomes `─`.
4. `/` → `"cap end arrow"` Enter — `▶` appears at target.
5. `/` → `"larger"` Enter twice — connection glyphs grow; "Smaller" reappears in the palette once the clamp isn't at max.
6. `/` → `"wide"` Enter — more space between glyphs.
7. `/` → `"accent"` Enter — color switches to `var(--accent)`.
8. `/` → `"convert to cross"` Enter — edge type flips, selection stays on the same edge under its new EdgeRef.
9. `/` → `"edit label"` Enter — static caret `▌` appears at the midpoint of the edge.
10. Type `beloved`, Enter — label persists on the edge.
11. Click directly on the rendered `beloved` label — inline editor reopens with text prefilled. Backspace to `bel`, type `ss`, Enter — label is now `bless`.
12. `/` → `"position label at start"` Enter — label jumps to the from-anchor end of the path.
13. `/` → `"clear label"` Enter — label disappears; the action self-hides on the next palette open.
14. `/` → `"reset connection style"` Enter — edge returns to the canvas default style; the per-edge override is cleared.
15. Ctrl+Z repeatedly — every step reverses cleanly, ending at the pre-session edge state.
16. Switch themes by clicking a theme-variant button in `theme_demo.mindmap.json` — any edge whose color was set to `var(--accent)` automatically restyles.

### Session 6E: Portal creation and management

**What**: Portals — matching glyph markers placed on two distant
nodes so the user can visually link them without drawing a connection
line across the canvas. Portals are a lightweight alternative to a
cross-link edge when two endpoints are far enough apart that a
rendered line would clutter the map. Each pair contributes *two*
marker glyphs (one per endpoint) that share the same glyph, color,
and label. Labels auto-assign in column-letter order ("A", "B", ...,
"Z", "AA", "AB", ...) picking the lowest unused letter so deleting
"B" and creating a new portal reuses "B". Default glyphs rotate
through an 8-entry preset palette so successive portals look
distinct at creation time.

- [x] "Create Portal" action: with two nodes Multi-selected, the `/`
      palette action "Create portal between selected nodes" fires
      `MindMapDocument::apply_create_portal`, pushes a `CreatePortal`
      undo entry, and pivots the selection to `SelectionState::Portal(pref)`
      so the follow-up glyph/color actions target the new pair
      without a click
- [x] Auto-assign labels (A, B, C, ..., AA, ...) via
      `MindMap::next_portal_label`, which scans `portals` for the
      lowest unused label in column-letter order (Excel-style);
      matching glyphs rotate through `PORTAL_GLYPH_PRESETS` =
      `["◈", "◆", "⬡", "⬢", "◉", "❖", "✦", "✧"]` indexed by
      `portals.len() % 8`
- [x] Portal glyphs rendered as markers on both endpoint nodes via a
      new `PortalElement` + `portal_elements: Vec<PortalElement>` on
      `RenderScene` (replacing the pre-existing empty
      `PortalElement {}` stub). The scene builder emits two elements
      per pair in a post-pass inside `build_scene_with_cache`,
      positioned above the top-right corner of each endpoint with a
      loose square AABB sized from the portal's `font_size_pt`.
      Portal colors resolve through `resolve_var` so
      `var(--accent)` auto-restyles on theme swap. Endpoints
      missing or hidden by fold skip the entire pair.
- [x] Select and delete portal pairs: click on a marker glyph →
      `Renderer::hit_test_portal` returns a `PortalRefKey` →
      `handle_click` routes to `SelectionState::Portal(PortalRef)`
      (mutually exclusive with node/edge selection, mirroring the
      `SelectionState::Edge` path). Delete key removes via
      `apply_delete_portal` which pushes a `DeletePortal` undo entry.
      Selected portals render in the cyan highlight color via the new
      `selected_portal` threaded through
      `build_scene_with_offsets_and_selection` / `build_scene_with_cache`
      alongside the existing `selected_edge` parameter.
- [x] Edit portal glyph symbols and colors via 8 palette actions:
      glyph presets "Portal glyph: hexagon/diamond/star/circle" and
      theme-var-aware colors "Portal color: accent/edge/fg/reset".
      Self-hide when the current glyph/color already matches.
      `set_portal_glyph` and `set_portal_color` wrap
      `apply_edit_portal` (a closure-based "snapshot, mutate, push
      `EditPortal` undo" chokepoint that mirrors the Session 6D
      `apply_edit_*` pattern).
- [x] Undo support for portal operations via three new variants:
      `UndoAction::CreatePortal { index }` /
      `UndoAction::DeletePortal { index, portal }` /
      `UndoAction::EditPortal { index, before }`. All three arms are
      direct clones of the corresponding `*Edge` variants'
      insert/remove/replace-in-place handling. Undoing a
      `CreatePortal` on the currently-selected portal also clears the
      selection so the UI doesn't hold a dangling reference.
- [x] 34 new tests (`./test.sh` green at 453 tests, +34 over the 419
      baseline): 8 model tests (PortalPair round-trip,
      column-letter label sequence, next_portal_label reuses gaps,
      wraps to AA after Z, glyph presets are unique + non-empty,
      backward-compat empty vec on load/serialize), 7 scene-builder
      tests (two-elements-per-pair, missing-endpoint and
      hidden-by-fold skip, theme-var resolution, selection color
      override, above-top-right placement, drag-offset tracking),
      12 document tests (create success, rejects
      self/unknown-node, sequential label assignment A/B/C, rotating
      glyph presets, undo create/delete/edit, delete reuses gap,
      selection mutual exclusivity with edge), and 7 palette tests
      (10 new action ids resolve, `two_nodes_selected` / 
      `portal_selected` applicability gating, `exec_create_portal`
      pivots selection, `exec_portal_glyph_hexagon` writes the
      field, `exec_delete_portal` clears selection, glyph-action
      self-hides when the current glyph already matches).

**Fork semantic**: portal mutations skip the Session 6D "fork on
first edit" pattern entirely because portals have no canvas-level
default to inherit from — each `PortalPair` always owns its own
concrete field values. `apply_edit_portal` just snapshots, mutates,
and records `EditPortal` without an intermediate fork step.

**Key files**:
- `lib/baumhard/src/mindmap/model.rs` — `PortalPair` struct,
  `MindMap.portals`, `next_portal_label`, `column_letter_label`,
  `PORTAL_GLYPH_PRESETS`, `default_portal_font_size`
- `lib/baumhard/src/mindmap/scene_builder.rs` — expanded
  `PortalElement`, new `PortalRefKey`, `selected_portal` parameter
  threaded through `build_scene` / `build_scene_with_offsets` /
  `build_scene_with_offsets_and_selection` / `build_scene_with_cache`,
  portal emission post-pass, `SELECTED_PORTAL_COLOR_HEX`
- `src/application/document.rs` — `PortalRef` struct,
  `SelectionState::Portal` variant, `UndoAction::{Create,Delete,Edit}Portal`
  variants + undo arms, `apply_create_portal` / `apply_delete_portal`
  / `apply_edit_portal` / `set_portal_glyph` / `set_portal_color`,
  `build_scene_with_selection` / `build_scene_with_cache` threading
  the new `selected_portal` arg
- `src/application/renderer.rs` — `portal_buffers` + `portal_hitboxes`
  `FxHashMap<(PortalRefKey, String), ...>` fields, `rebuild_portal_buffers`,
  `hit_test_portal`, `portal_buffers.values()` chained into the main
  text-area render pass next to `connection_label_buffers.values()`
- `src/application/app.rs` — `handle_click` portal fallthrough
  before the edge hit test, Delete-key handler extended to route
  `SelectionState::Portal` through `apply_delete_portal`, shift-click
  selection-carry path extended to treat `Portal(_)` like `Edge(_)`,
  `rebuild_portal_buffers` added at every existing
  `rebuild_connection_label_buffers` call site (8 sites including
  the hot drag drain and the camera-change rebuild)
- `src/application/palette.rs` — 10 new predicates + 10 executors +
  10 `PaletteAction` entries under a new "Session 6E" banner, new
  test count assertion (42 → 52)

**Verify**: `./test.sh` green at 453 tests.
`cargo run -- maps/testament.mindmap.json`:

1. Click a node, shift+click another — two nodes in `Multi`
   selection (cyan highlights on both).
2. `/` → "Create portal" → Enter — two matching `◈` glyphs appear
   above the top-right corner of both nodes, labeled "A". Selection
   pivots to the new portal (both markers render in cyan).
3. `/` → "Portal glyph: hexagon" → Enter — both markers become
   `⬡`. The "hexagon" action self-hides from the next palette open.
4. `/` → "Portal color: accent" → Enter — markers resolve through
   `var(--accent)` and pick up the theme color.
5. Click either marker again to confirm hit-testing by glyph.
6. Delete key — both markers vanish, selection clears.
7. Ctrl+Z — portal restored with its original glyph and color,
   selection clears (LIFO undo order).
8. Ctrl+Z again — the create is undone; portals vec is empty.
9. Create three portals in sequence against different node pairs —
   confirm labels progress to "A", "B", "C". Delete "B". Create
   another — confirm the new label is "B" (gap reuse).
10. Switch themes on `theme_demo.mindmap.json` — any
    `var(--accent)`-colored portal auto-restyles.

---

## Milestone 7: Text Editing

**Goal**: Edit node text content inline. Text changes modify the GlyphArea in the Baumhard tree, then sync back to the MindMap model.

### Session 7A: Inline text editor and node creation

**What**: Double-click to edit node text, create new nodes via empty-canvas double-click. Multi-line paragraph editor — Enter/Tab are literal characters, not outline gestures.

- [x] Double-click node → enter edit mode with cursor (caret glyph via Baumhard `Mutation::AreaDelta { text: Assign }`, applied directly to the live tree's `GlyphArea`)
- [x] Text input, deletion, char-level cursor movement (Left/Right/Home/End, Up/Down with simple `\n`-based line tracking). Text selection (shift+arrow) deferred to 7B.
- [ ] ~~Rich text: Ctrl+B bold, Ctrl+I italic~~ — deferred to 7B. Multi-run nodes collapse to a single run on first edit.
- [x] Esc → cancel (rebuild from untouched model). Click outside the edited node → commit via `set_node_text` which pushes `UndoAction::EditNodeText`.
- [x] Double-click empty space → create new orphan node via `apply_create_orphan_node`, push `CreateNode` undo, select, open editor with empty buffer (the model retains `"New node"` as fallback if the user cancels).
- [ ] ~~Tab from selected node → create child node~~ — not pursued. Per user direction ("we are not aiming to reproduce any sort of classic workflow"), Tab and Enter are literal characters inside the editor. Node creation outside the editor is via double-click empty canvas + Ctrl+N + Ctrl+P reparent.
- [ ] ~~Enter from selected node → create sibling node~~ — not pursued, same rationale.

**Key files** (Session 7A additions):
- `src/application/app.rs` — new `TextEditState` enum, `LastClick` struct, pure helpers (`insert_at_cursor`, `delete_before_cursor`, `delete_at_cursor`, `cursor_to_line_start`, `cursor_to_line_end`, `move_cursor_up_line`, `move_cursor_down_line`, `insert_caret`, `is_double_click`), `open_text_edit`, `close_text_edit`, `apply_text_edit_to_tree`, `handle_text_edit_key`. New `#[cfg(test)] mod text_edit_tests` with 25+ unit tests covering cursor math, caret insertion, double-click window, and a Baumhard `DeltaGlyphArea` round-trip that asserts text edits really do flow through `Mutation::apply_to(area)`.
- `src/application/document.rs` — new `UndoAction::EditNodeText { node_id, before_text, before_runs }` variant + undo arm. New `set_node_text` method mirroring `set_edge_label`'s contract (no-op on unchanged, collapses runs on commit, one undo push per committed edit). 6 new unit tests.
- No Baumhard library changes — `Mutation::AreaDelta` + `DeltaGlyphArea::new(vec![GlyphAreaField::Text, GlyphAreaField::Operation(Assign)])` + `GfxElement::glyph_area_mut()` already cover everything.

**Verify**: `./test.sh` green (511 tests, +38 new). `cargo run -- maps/testament.mindmap.json`:
1. Double-click a node → caret appears at end of text; type, Backspace, arrows move cursor, Home/End jump within a line.
2. Press Enter → new line inserted inline (multi-line edit).
3. Press Tab → tab character inserted.
4. Press Esc → edit discarded; node reverts.
5. Double-click, type, click a different node → committed; Ctrl+Z restores original.
6. Double-click empty canvas → new orphan node with caret; type, click outside → committed; Ctrl+Z restores "New node"; Ctrl+Z removes the orphan.
7. Open palette `/` mid-edit → key is typed, not intercepted; keyboard-steal precedence holds.
8. Edge label editor (Session 6D) still works unaffected.

### Session 7A follow-up: Keyboard entry + node deletion + regions fix

**What**: Session 7A shipped the inline editor but left three gaps
that surfaced as soon as the feature was used in anger — one silent
bug and two missing features.

- [x] **Bug fix: double-click on an existing multi-run node now shows
  the caret and typed text.** `apply_text_edit_to_tree` at
  `src/application/app.rs:2574` previously sent a
  `DeltaGlyphArea { Text: Assign }` without touching
  `ColorFontRegions`. The renderer at `src/application/renderer.rs:1500-1520`
  only shapes characters that fall inside at least one region, so
  the caret glyph appended at `text.len()` (and any subsequent typed
  chars past the original length) were silently dropped. Fix:
  replace regions with a single region `[0, display_char_count)`
  that inherits the color of the first pre-edit region, applied via
  the same `Assign` delta. New regression test
  `test_text_edit_replaces_stale_regions_to_cover_caret` in
  `text_edit_tests` proves the round-trip.
- [x] **Enter on a selected single node opens the editor with
  existing text, cursor at end.** New `Action::EditSelection` in
  `src/application/keybinds.rs` + match arm in the keyboard dispatch
  at `src/application/app.rs`. Default keybind `Enter`. Editor still
  steals keys at the top of dispatch, so Enter-inside-editor stays a
  literal newline.
- [x] **Backspace on a selected single node opens the editor with an
  empty buffer — "clean slate" retype.** New
  `Action::EditSelectionClean` with default keybind `Backspace`.
  Reuses `open_text_edit(..., from_creation = true)` which already
  starts with `String::new()`; commit replaces the text via
  `set_node_text` + `EditNodeText` undo push, no new undo variant
  needed.
- [x] **Delete on a selected node deletes it and orphans its immediate
  children.** New `MindMapDocument::delete_node` at
  `src/application/document.rs` that removes the node, orphans each
  direct child (setting `parent_id = None` with fresh root indices),
  and removes every parent_child / cross_link edge that touched it.
  New `UndoAction::DeleteNode { node, removed_edges, orphaned_children }`
  variant fully reverses the operation. `Action::DeleteSelection`
  handling at `src/application/app.rs` extended to cover
  `SelectionState::Single(..)` and `SelectionState::Multi(..)` —
  previously explicitly scoped out as a future milestone.

**Key files** (follow-up additions):
- `src/application/keybinds.rs` — `Action::EditSelection`,
  `Action::EditSelectionClean` + `KeybindConfig.edit_selection` /
  `.edit_selection_clean` fields + default bindings (`Enter`,
  `Backspace`) + extended `test_default_config_has_all_actions`.
- `src/application/document.rs` — `delete_node` method,
  `UndoAction::DeleteNode` variant + undo arm, five new tests
  (`test_delete_node_orphans_children`,
  `test_delete_node_removes_all_touching_edges`,
  `test_delete_node_undo_restores_node_edges_and_children`,
  `test_delete_node_missing_returns_none`,
  `test_delete_root_node_works`).
- `src/application/app.rs` — `apply_text_edit_to_tree` regions fix,
  new `EditSelection` / `EditSelectionClean` match arms, extended
  `DeleteSelection` match with `Node` / `Nodes` branches, new
  regression test for the regions fix.

**Verify**: `./test.sh` green (537 tests, +26 new).
`cargo run -- maps/testament.mindmap.json`:
1. Double-click an existing node with styled text → caret visible,
   typing updates live, click-outside commits, Ctrl+Z restores.
2. Select a node, press Enter → editor opens at end of existing text.
3. Select a node, press Backspace → editor opens with empty buffer;
   type "foo", click outside → node text becomes "foo"; Ctrl+Z
   restores original text.
4. Select a node with children, press Delete → node removed;
   children become roots at same positions; Ctrl+Z restores node,
   edges, and children's original parent_id + index.

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
