# Portal labels — per-endpoint state

Portal-mode edges (`edge.display_mode = "portal"`) render as two
glyph labels, one attached to each endpoint node's border. Each
label has its own color override and its own position along the
owning node's border, encoded on the edge as the `portal_from` /
`portal_to` fields.

Both fields are optional and absent when default. An edge freshly
flipped to portal mode has neither field set; each label inherits
from the edge's base color and auto-orients toward its partner.

## Fields

```jsonc
{
  "from_id": "a",
  "to_id":   "b",
  "type":    "cross_link",
  "display_mode": "portal",
  "color":   "#aa88cc",
  // ...
  "portal_from": {
    "color":    "#ff8800",   // per-endpoint color override
    "border_t": 1.5          // pinned position on a's border
  },
  "portal_to": {
    "color": "var(--accent)"
  }
}
```

- `portal_from` — state for the label attached to `from_id`'s
  node.
- `portal_to` — state for the label attached to `to_id`'s node.

Each side has the same shape:

| Field                    | Type                                    | Default   | Meaning                                                            |
| ------------------------ | --------------------------------------- | --------- | ------------------------------------------------------------------ |
| `color`                  | `#rrggbb` / `#rrggbbaa` / `var(--name)` | inherit   | Icon color override for this label only.                           |
| `border_t`               | `f32`, `[0, 4)`                         | auto      | Position along the owning node's border.                           |
| `text`                   | string                                  | absent    | Text label rendered next to the icon.                              |
| `text_color`             | `#rrggbb` / `#rrggbbaa` / `var(--name)` | inherit   | Text color override — independent from the icon `color`.           |
| `text_font_size_pt`      | `f32`                                   | inherits  | Target on-screen text size at zoom 1.0; falls back to icon size.   |
| `text_min_font_size_pt`  | `f32`                                   | inherits  | Lower screen-space clamp for the text; falls back to the edge's.   |
| `text_max_font_size_pt`  | `f32`                                   | inherits  | Upper screen-space clamp for the text; falls back to the edge's.   |

The `text` field — when present — renders as a sibling glyph
area next to the portal marker icon, positioned outward of the
icon along the border normal so the text extends away from the
owning node (never back toward it).

The icon and the text each carry their own color + size channels,
and cascade independently: `text_color` → icon color cascade
(`color` → `glyph_connection.color` → `edge.color`), and
`text_font_size_pt` → `glyph_connection.font_size_pt`. The two
channels let a coloured badge carry a differently-coloured
annotation beside it — parity with line-mode edge labels, which
similarly detach from the edge body color.

Authored via `label text="…"` or `label edit` on a portal-label
selection (same console verbs that author edge labels —
dispatch splits on the current selection variant).

## Color cascade

Highest priority wins:

1. `portal_from` / `portal_to.color` — per-endpoint override.
2. `edge.glyph_connection.color` — edge-level override.
3. `edge.color` — the edge's always-present base color.

The wheel color picker, `color` console verb, paste, and cut all
route to the per-endpoint field when a portal label is selected,
and to `edge.color` / `glyph_connection.color` when the whole edge
or canvas is selected.

## Position — `border_t`

`border_t` is a scalar perimeter parameter on the owning node's
rectangular border. Canonical range is `[0.0, 4.0)` — one unit per
side, walked clockwise from the top-left corner:

```text
  0 ────────→ 1
  ↑           ↓
  3           1
  ↑           ↓
  3 ←──────── 2
```

- `t ∈ [0, 1)` → top edge, left → right
- `t ∈ [1, 2)` → right edge, top → bottom
- `t ∈ [2, 3)` → bottom edge, right → left
- `t ∈ [3, 4)` → left edge, bottom → top

Using a side-indexed parameter (rather than a single perimeter
fraction that stretches with aspect ratio) keeps the label's
apparent position stable when the owning node is resized.

**Auto (field absent):** the renderer computes the ray from the
owner's center through the partner's center and anchors the label
where that ray exits the border. A label with no `border_t` thus
always *faces* its partner, even as either node moves.

**Pinned (field present):** the user dragged the label to that
specific perimeter parameter. Subsequent partner-position changes
do not reflow the label.

## Load-time behavior

- Both fields have serde defaults of `None`, so existing maps
  without them load cleanly.
- Missing or invalid nested fields fall back to the next cascade
  stage (no strict schema errors).
- `border_t` values outside `[0, 4)` wrap into range on first
  use — the drag path produces canonical values; hand-edited JSON
  can't crash the renderer with a value of, say, `7.3`.

## Related

- `enums.md` — `edge.display_mode` top-level documentation.
- `channels.md` — rendering channel layout (portal labels ride on
  the same scene tree as line-mode connections).
