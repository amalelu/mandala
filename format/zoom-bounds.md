# Zoom-Visibility Bounds

A zoom-visibility window gates whether a renderable **appears at all**
at the current `camera.zoom`, independent of the size-clamp machinery
that reshapes fonts to stay legible (`min_font_size_pt` /
`max_font_size_pt`). This is the primitive map authors use to build
Google-Maps-style layered detail — landmark labels that only show at
overview zoom, or fine-grained annotations that only show when zoomed
in.

Every `.mindmap.json` type that can carry a window exposes the same
flat pair of optional fields:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `min_zoom_to_render` | `f32 \| null` | `null` | Lower inclusive bound on `camera.zoom`. `null` = unbounded below. |
| `max_zoom_to_render` | `f32 \| null` | `null` | Upper inclusive bound on `camera.zoom`. `null` = unbounded above. |

Both `null` — the default — means the element renders at every zoom.
A one-sided window (only `min` or only `max` set) is fully supported.

## Where it attaches

| Type | Effect when set |
|---|---|
| [`MindNode`](./schema.md#node) | Gates the node text, background fill, and glyph border as one unit |
| [`MindEdge`](./schema.md#edge) | Gates the edge body, caps, label (unless overridden), and both portal endpoints (unless overridden) |
| [`EdgeLabelConfig`](./schema.md#edgelabelconfig) | Per-label override on a line-mode edge |
| [`PortalEndpointState`](./portal-labels.md) | Per-endpoint override on a portal-mode edge (icon + text render as one unit) |

## Cascade: replace, not intersect

When a label or portal endpoint declares **either** bound, its pair
fully **replaces** the parent edge's pair. Intersection would silently
inherit a bound the author didn't mention — setting only
`min_zoom_to_render` on a label and not `max_zoom_to_render` should
mean "override with this one-sided window", not "narrow the edge's
window further".

Same posture the portal font-clamp resolver already uses for
`text_{min,max}_font_size_pt`.

```text
# Edge with a closed window, label with a one-sided override:
MindEdge        { min: 0.5, max: 2.0 }
EdgeLabelConfig { min: 1.5, max: null }   // label overrides

# Resolved:
edge body:   { min: 0.5,  max: 2.0 }
label:       { min: 1.5,  max: null }     // replace, not intersect
```

## Validation

`maptool verify` rejects `min > max` (when both are `Some`) under the
`zoom_bounds` category. The runtime `ZoomVisibility::contains` check
still returns cleanly for an inverted window — it just always reports
"not visible" — but the authoring intent is almost always a typo.

## Mutator target

Zoom windows are mutable through the Baumhard tree mutator pipeline
via `GlyphAreaField::ZoomVisibility(...)` — see
[mutations.md](./mutations.md). A `CustomMutation` can swap a node's
window at runtime, which is the seam for zoom-triggered LOD
transitions ("at zoom 2×, swap this cluster into its detail view").

## Console

Authored at runtime through the `zoom` console command (alias:
`visibility`):

```
zoom min=1.5 max=3.0      # set both bounds on the current selection
zoom min=0.5              # set only min, leave max untouched
zoom max=unset            # clear just the max side (back to unbounded above)
zoom clear                # clear both bounds
```

Routing mirrors the `font` command:

| Selection | Target struct |
|---|---|
| `Single(node)` | `MindNode.{min,max}_zoom_to_render` |
| `Multi(nodes)` | fans out over each node |
| `Edge(edge)` | `MindEdge.{min,max}_zoom_to_render` |
| `EdgeLabel(label)` | `EdgeLabelConfig.{min,max}_zoom_to_render` (replace cascade) |
| `PortalLabel(icon)` | owning edge's top-level pair (icon inherits edge) |
| `PortalText(text)` | `PortalEndpointState.{min,max}_zoom_to_render` (replace cascade) |

Values must be positive and finite; `unset` / empty string clears
the side back to `None` (unbounded). Inverted bounds are rejected
with an error before any model state changes.

## Cost

The render-loop cull is two branchless `Option<f32>` comparisons per
`GlyphArea` per frame (bench: `zoom_visibility_contains` — sub-ns on
the current hardware). No text reshaping or buffer-cache invalidation
fires on zoom steps — the cull runs against cached buffers.
