# MindMap JSON Format Specification

Version: 1.0

This document specifies the `.mindmap.json` format produced by the miMind
converter. It is designed for consumption by a Rust/WASM rendering client.

## Design Principles

- **Flat node map** keyed by ID (avoids recursive ownership in Rust)
- **Separate edges array** as first-class objects with full styling
- **Lossless conversion** from miMind: all visual and structural data preserved
- **Hex colors** (`#RRGGBB`) converted from miMind's RGBA format
- **Positions in pixels** matching miMind's absolute canvas coordinate system

## Top-Level Structure

```json
{
  "version": "1.0",
  "name": "map-name",
  "canvas": {
    "background_color": "#141414"
  },
  "nodes": { ... },
  "edges": [ ... ]
}
```

| Field | Type | Description |
|-------|------|-------------|
| `version` | string | Format version (currently "1.0") |
| `name` | string | Map name (derived from input filename) |
| `canvas.background_color` | string | Canvas background as `#RRGGBB` |
| `nodes` | object | Map of node ID (string) to Node object |
| `edges` | array | Array of Edge objects |

## Node Object

```json
{
  "id": "348068464",
  "parent_id": null,
  "index": 250,
  "position": { "x": -179.43, "y": -2413.80 },
  "size": { "width": 373.52, "height": 109.0 },
  "text": "Lord God",
  "text_runs": [ ... ],
  "style": { ... },
  "layout": { ... },
  "folded": false,
  "notes": "",
  "color_schema": null
}
```

### Node Fields

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Unique node identifier |
| `parent_id` | string \| null | Parent node ID, `null` for root nodes |
| `index` | int | Sibling ordering (ascending) |
| `position.x` | float | Absolute X canvas coordinate |
| `position.y` | float | Absolute Y canvas coordinate |
| `size.width` | float | Node width in pixels |
| `size.height` | float | Node height in pixels |
| `text` | string | Plain text content (may contain newlines) |
| `text_runs` | array | Rich text formatting runs (see below) |
| `style` | object | Visual styling (see below) |
| `layout` | object | Child layout configuration (see below) |
| `folded` | bool | `true` if children are collapsed |
| `notes` | string | Note text (empty string if none) |
| `color_schema` | object \| null | Color theme info, `null` if none |

### Text Run Object

Each text run represents a span of text with uniform formatting. Runs are
ordered and contiguous; together they cover the full `text` string.

```json
{
  "start": 0,
  "end": 8,
  "bold": true,
  "italic": false,
  "underline": false,
  "font": "LiberationSans",
  "size_pt": 74,
  "color": "#ffffff",
  "hyperlink": null
}
```

| Field | Type | Description |
|-------|------|-------------|
| `start` | int | Start character offset (inclusive) |
| `end` | int | End character offset (exclusive) |
| `bold` | bool | Bold formatting |
| `italic` | bool | Italic formatting |
| `underline` | bool | Underline formatting |
| `font` | string | Font family name |
| `size_pt` | int | Font size in points |
| `color` | string | Text color as `#RRGGBB` |
| `hyperlink` | string \| null | URL if this run is a hyperlink |

### Style Object

```json
{
  "background_color": "#141414",
  "frame_color": "#30b082",
  "text_color": "#ffffff",
  "shape_type": 5,
  "corner_radius_percent": 10.0,
  "frame_thickness": 4.0,
  "show_frame": false,
  "show_shadow": false
}
```

| Field | Type | Description |
|-------|------|-------------|
| `background_color` | string | Node background fill (`#RRGGBB`, empty if transparent) |
| `frame_color` | string | Node border color (`#RRGGBB`, empty if none) |
| `text_color` | string | Default text color (`#RRGGBB`, empty if inherited) |
| `shape_type` | int | Shape type identifier from miMind |
| `corner_radius_percent` | float | Corner rounding as percentage (0-100) |
| `frame_thickness` | float | Border width in pixels |
| `show_frame` | bool | Whether the border is drawn |
| `show_shadow` | bool | Whether a drop shadow is drawn |

### Layout Object

Defines how this node's **children** are arranged.

```json
{
  "type": 0,
  "direction": 6,
  "spacing": 50.0
}
```

| Field | Type | Description |
|-------|------|-------------|
| `type` | int | Layout algorithm (see enum below) |
| `direction` | int | Branch direction (see enum below) |
| `spacing` | float | Gap between siblings in pixels |

#### Layout Type Enum

| Value | Name | Description |
|-------|------|-------------|
| 0 | Map | Free/scattered 2D canvas placement |
| 1 | Tree | Structured: fixed distance on branch axis, spread on perpendicular axis |
| 2 | Outline | Linear list at fixed distance from parent |

#### Layout Direction Enum

| Value | Name | Tree behavior | Map behavior |
|-------|------|---------------|--------------|
| 0 | Auto | - | Free placement |
| 1 | Up | Horizontal row above (fixed dy, varying dx) | Biased upward |
| 2 | Down | Horizontal row below (fixed dy, varying dx) | Biased downward |
| 3 | Left | Vertical column left (fixed dx, varying dy) | Biased leftward |
| 4 | Right | Vertical column right (fixed dx, varying dy) | Biased rightward |
| 6 | Balanced | - | Radial scatter, even L/R split |

### Color Schema Object

Present when a node participates in a color theme. `null` if no theme applies.

```json
{
  "level": 0,
  "palette": "coral",
  "variant": 3,
  "starts_at_root": true,
  "connections_colored": true,
  "theme_id": "Pastel:#BFFFFFFE01",
  "groups": [
    {
      "background": "#a9decb",
      "frame": "#30b082",
      "text": "#000000",
      "title": "#000000"
    }
  ]
}
```

| Field | Type | Description |
|-------|------|-------------|
| `level` | int | Node's depth from schema root (0 = root of theme subtree) |
| `palette` | string | Palette name (e.g., "coral", "sweet", "sandy") |
| `variant` | int | Palette variant (2 = standard, 3 = alternate) |
| `starts_at_root` | bool | Whether level counting starts from schema root |
| `connections_colored` | bool | Whether edges inherit theme colors |
| `theme_id` | string | Theme identifier (e.g., "Pastel:#BFFFFFFE01") |
| `groups` | array | Color groups indexed by depth (only present on schema root nodes) |

#### Color Group Object

Each group defines colors for one depth level within the themed subtree.

| Field | Type | Description |
|-------|------|-------------|
| `background` | string | Node background fill `#RRGGBB` |
| `frame` | string | Node border color `#RRGGBB` |
| `text` | string | Body text color `#RRGGBB` |
| `title` | string | Title text color `#RRGGBB` |

**Palette inheritance**: Only the schema root (level 0) includes the `groups`
array. Deeper nodes (level 1, 2, ...) reference the same palette by name
but have an empty `groups` array. The renderer should look up the schema root
to find the ColorGroup at the appropriate index.

## Edge Object

```json
{
  "from_id": "348068464",
  "to_id": "351582192",
  "type": "parent_child",
  "color": "#30b082",
  "width": 6,
  "line_style": 0,
  "visible": true,
  "label": null,
  "anchor_from": 0,
  "anchor_to": 3,
  "control_points": []
}
```

| Field | Type | Description |
|-------|------|-------------|
| `from_id` | string | Origin node ID |
| `to_id` | string | Target node ID |
| `type` | string | `"parent_child"` or `"cross_link"` |
| `color` | string | Line color `#RRGGBB` |
| `width` | int | Line thickness in pixels |
| `line_style` | int | Line style (0 = default, 1 = alternative) |
| `visible` | bool | Whether the connection is drawn |
| `label` | string \| null | Connection label text, `null` if none |
| `anchor_from` | int | Attachment point on origin node (0 = auto, 3 = specific) |
| `anchor_to` | int | Attachment point on target node |
| `control_points` | array | Bezier control points for curved connections |

### Control Point Object

```json
{ "x": -597.61, "y": 112.67 }
```

Ordered list of intermediate points that define the curve of the connection.
Empty array for straight connections.

## Glyph Border Configuration (v1.1 extension)

All glyph rendering extensions are **optional**. Existing v1.0 files continue
to load unchanged — the renderer applies tasteful defaults when properties are
absent.

### Canvas-level Defaults

The `canvas` object may include default glyph styles applied to all nodes/edges:

```json
{
  "canvas": {
    "background_color": "#141414",
    "default_border": { ... },
    "default_connection": { ... }
  }
}
```

### GlyphBorderConfig (on `style.border`)

```json
{
  "preset": "rounded",
  "font": null,
  "font_size_pt": 14.0,
  "color": null,
  "padding": 4.0,
  "glyphs": null
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `preset` | string | `"rounded"` | Glyph preset: `"light"`, `"heavy"`, `"double"`, `"rounded"`, `"custom"` |
| `font` | string \| null | null | Font family for border glyphs (null = system default) |
| `font_size_pt` | float | 14.0 | Font size in points |
| `color` | string \| null | null | Border color `#RRGGBB` (null = inherit `frame_color`) |
| `padding` | float | 4.0 | Padding between border and node content (pixels) |
| `glyphs` | object \| null | null | Custom glyph definitions (only when preset = "custom") |

### CustomBorderGlyphs (on `style.border.glyphs`)

```json
{
  "top": "─", "bottom": "─",
  "left": "│", "right": "│",
  "top_left": "╭", "top_right": "╮",
  "bottom_left": "╰", "bottom_right": "╯"
}
```

Each field is a string containing the glyph(s) for that border segment.
Multi-character strings are allowed for decorative borders.

### GlyphConnectionConfig (on edge `glyph_connection`)

```json
{
  "body": "·",
  "cap_start": null,
  "cap_end": "→",
  "font": null,
  "font_size_pt": 12.0,
  "min_font_size_pt": 8.0,
  "max_font_size_pt": 24.0,
  "color": null,
  "spacing": 0.0
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `body` | string | `"·"` | Glyph(s) repeated along the connection path |
| `cap_start` | string \| null | null | Glyph at the source end |
| `cap_end` | string \| null | null | Glyph at the target end (e.g. arrowhead) |
| `font` | string \| null | null | Font family (null = system default) |
| `font_size_pt` | float | 12.0 | Target on-screen glyph size at `camera.zoom == 1.0`. At other zoom levels the canvas-space font size is derived from this base and clamped — see note below. |
| `min_font_size_pt` | float | 8.0 | Lower bound on the on-screen glyph size. Prevents glyphs from collapsing into an unreadable dust cloud at extreme zoom-out; the canvas-space size inflates to keep on-screen size ≥ this value, which also reduces the per-edge glyph count. |
| `max_font_size_pt` | float | 24.0 | Upper bound on the on-screen glyph size. Prevents glyphs from rendering as a few enormous boulders at extreme zoom-in; the canvas-space size shrinks to compensate, so more densely-sampled glyphs follow the path. |
| `color` | string \| null | null | Color `#RRGGBB` (null = inherit edge `color`) |
| `spacing` | float | 0.0 | Gap between repeated body glyphs (0 = tight) |

The effective canvas-space font size at any given zoom is
`clamp(font_size_pt * camera.zoom, min_font_size_pt, max_font_size_pt) / camera.zoom`.
Sample spacing along the connection path is proportional to that
effective size, so glyph count rises and falls automatically as the
user zooms — fewer-but-larger glyphs when zoomed out, more-but-smaller
glyphs when zoomed in.

## Coordinate System

- Origin (0, 0) is an arbitrary point on the canvas
- X increases to the right
- Y increases downward (screen coordinates)
- All positions are absolute canvas coordinates, not relative to parent
- The renderer must compute its own viewport/camera from node positions
