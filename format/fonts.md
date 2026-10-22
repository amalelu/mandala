# Fonts

Mandala bundles every font it ships with, so the binary renders any
saved map without external assets. The intent is that what's compiled
in is *all* that is available, on every target: identical glyphs on
native and in the browser (CODE_CONVENTIONS §4), and a map that looks
the same wherever it is opened.

**The native build does not yet honor that.**
`baumhard::font::fonts`' `FONT_SYSTEM` is
`RwLock::new(FontSystem::new())`, and cosmic-text's `FontSystem::new`
indexes the host's fontconfig — so on native, `font list`,
`font set <family>` and the completion popup all offer whatever the
machine happens to have installed, attributed to `AppFont::Any`,
alongside the compiled-in faces. The browser build has no fontconfig
to index, so it really does see only what is compiled in. A map
authored on a Linux desktop against a host family therefore renders
in the monospace fallback in a browser, with a `warn!`, and nothing
about the file says why.

Closing the gap means constructing the `FontSystem` without the
system sources. That is deliberately **not** done here: it changes
what every existing map renders as on native — including
`maps/testament.mindmap.json`, which pins families no compiled-in
face provides — so it is a visual-regression decision with its own
migration, not a line change. Until it is taken, read the paragraph
above as: the compiled-in set is the *portable* set, and anything
outside it is a native-only accident. New surfaces should assume the
portable set; `baumhard::mindmap::placard` is the worked example, and
its `PLACARD_FONT_FAMILY` doc comment explains why it pins nothing at
all rather than naming a family that exists on one target.

## Where fonts are pinned in the data model

Two fields hold the user's font choice:

- **`MindNode.text_runs[*].font`** (`String`) — per-run pin on a
  node's text. See [`text-runs.md`](./text-runs.md).
- **`MindEdge.glyph_connection.font`**
  (`Option<String>`) — pin for the edge body's connection glyphs.
  When `null` (or absent), the edge inherits the canvas-level
  default connection font.

Edge labels, portal icons, and portal text **inherit** the owning
edge's font today — there is no per-channel `font_family` slot on
`EdgeLabelConfig` or `PortalEndpointState`. A future commit may
add those slots when the graphical font picker calls for it.

## Family-name semantics

Each value is a family-name string the renderer resolves through
`baumhard::font::fonts::app_font_by_family` to the build-time
`AppFont` enum. The `AppFont` is then carried on the
[`ColorFontRegion`](../lib/baumhard/src/core/primitives.rs) the
tree builder hands to the renderer, and pinned via the baumhard
attrs builder.

- **Empty string** on `TextRun.font` clears the pin (run uses the
  document default).
- **Null** on `GlyphConnectionConfig.font` clears the override
  (edge inherits the canvas default connection font).
- **Unknown family** at render time logs a `warn!` and falls back
  to monospace — the renderer never panics on a bad family
  (interactive-path invariant per CODE_CONVENTIONS §9).

## Console verbs

Three forms live under one `font` command:

```
font set <family>           # pin the family on the current selection
font list                   # list every loaded family, each in its face
font size=<pt> [min=<pt>] [max=<pt>]    # font-size + clamp triple
```

`font set <family>` mirrors the color-wheel pattern: it dispatches
through the `AcceptsFontFamily` trait on `TargetView` so each
component variant decides which channel a channel-less font choice
lands on.

| Selection variant | Channel `font set` writes |
|-------------------|---------------------------|
| `Node`            | every `TextRun.font` on the node |
| `Edge`            | `glyph_connection.font` on the edge |
| `PortalLabel`     | the owning edge's `glyph_connection.font` (icon shares edge body's font) |
| `EdgeLabel`       | not applicable (inherits the edge's font) |
| `PortalText`      | not applicable (inherits the edge's font) |

`font list` emits one scrollback line per loaded family, each
shaped in its own face — a quick visual reference for picking.
The console scrollback is scrollable (Shift+Up/Down, PgUp/PgDn,
mousewheel, Shift+Home/End) so the full list is reachable even
when it overflows the visible window.

## Completion

When the cursor sits in the family slot of `font set ...`, the
console completion popup shows every family whose name starts
with the typed prefix (case-insensitive), each candidate row
pre-shaped in that very face. Submitting a candidate writes the
canonical family-name string back to the data model.

## Cross-platform

The data-model fields, the `set_node_font_family` /
`set_edge_font_family` document setters, the `AcceptsFontFamily`
trait, and the `list_loaded_families` enumeration helper all
compile and apply on `wasm32-unknown-unknown`. The `font` console
verb itself is native-only by inheritance — the console UI is
native-only — but a future graphical font picker can attach to
the same primitives without re-doing the foundation.
