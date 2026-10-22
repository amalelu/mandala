# Text Runs

`text_runs` is a list of formatting spans applied to a
[`MindSection`](./sections.md)'s `text`. Post-section refactor
they live on the section, not on the node directly — every
`MindNode.sections[i].text_runs` is its own independent run table.

```json
{
  "sections": [
    {
      "text": "Hello world",
      "text_runs": [
        { "start": 0, "end": 5, "bold": true, "font": "LiberationSans", "size_pt": 14, "color": "#ffffff" }
      ]
    }
  ]
}
```

Each run covers a grapheme-cluster range `[start, end)` and carries
formatting metadata: `bold`, `italic`, `underline`, `font`, `size_pt`,
`color`, and `hyperlink`.

**Only `start` and `end` are required.** Every styling field defaults
to the value that renders like the uncovered text around it, so
`{ "start": 0, "end": 5, "bold": true }` is a complete run:

- `bold` / `italic` / `underline` default to `false`.
- `font` defaults to the empty string — no pin; the run uses the
  document default face (see [fonts.md](./fonts.md)).
- `size_pt` is a number (fractional sizes like `14.5` are valid) and
  defaults to `14`, the same scale a section with no runs at all
  renders at — omitting it changes nothing visually.
- `color` defaults to the empty string — the run declines to name a
  color and takes the node's effective text color; see
  ["Leaving `color` empty"](#leaving-color-empty) below.
- `hyperlink` defaults to absent.

The saver omits fields still sitting at their defaults, per the
omission policy in [schema.md](./schema.md) ("a key written out at
its own default value may be omitted when the map is saved") —
reloading the terse form restores the same run.

## Why runs are optional

`text_runs` defaults to an empty array when absent. A section with
empty runs renders its entire text using the owning node's base
style — the node's *effective* text color (its
`color_schema.overrides.text` when it names one, else the palette
group's `text` when the node is themed, else `style.text_color`;
see [palettes](./palettes.md)), a default font, a default size.

This matters for hand-authoring. A simple node becomes:

```json
"0": { "id": "0", "sections": [{"text": "Hello"}], "style": { "text_color": "#ffffff", ... }, ... }
```

No noise. No need to declare a run that just repeats the style.

## Coverage rules

When `text_runs` is non-empty:

- Runs must not overlap: each run's `end` must be `<= next.start`
  (which implies ascending `start` order for well-formed runs)
- `start < end` for every run
- `start` and `end` are measured in **grapheme clusters** — what users
  see as "one character" — not bytes and not Unicode code points
- Uncovered ranges inherit the node's base style (so partial coverage is
  valid — you can decorate just the first word without declaring runs for
  the rest)

Grapheme clusters match the unit baumhard's text primitives speak in
(see `lib/baumhard/CONVENTIONS.md §B1` and the helpers in
`lib/baumhard/src/util/grapheme_chad.rs`). The cosmic-text bridges in
`baumhard::font::attrs` slice text by grapheme too, so a run that ends
on a ZWJ-emoji or combining-mark cluster boundary lands on a UTF-8-valid
byte boundary and shapes correctly. The `text` field carries the
authored characters verbatim; clusters are the unit that stays stable
through round-trips and matches what `maptool verify` checks.

`maptool verify` checks all these invariants and reports specific
violations with run indices.

## Example: partial coverage

```json
{
  "text": "Hello world",
  "text_runs": [
    { "start": 0, "end": 5, "bold": true, ... }
  ]
}
```

"Hello" is bold. " world" inherits the node's effective text color,
default font, default size. Valid.

## Leaving `color` empty

`color` is the one run field with a **third** state. A run naming a
`#rrggbb` (or `var(--name)`) color owns those graphemes outright and
keeps them through a retheme. A run whose `color` is the empty
string declines to name one and takes the node's effective text
color, exactly as an uncovered range does — it is a formatting span
that happens to carry no color opinion.

The distinction is load-bearing, and it is why the authoring layer
writes an empty `color` on every run it creates with nothing to
inherit from (a text edit on a run-less section, a `bold` over a
range, a clipboard splice). Baking the currently-effective hex in
would render identically the same day and quietly opt those
graphemes out of the palette: the next palette edit would move every
sibling and leave them behind.

## Hyperlinks

A run can set `"hyperlink": "https://example.com"`. The renderer draws
the covered text as a clickable link styled with that URL. Runs without
a hyperlink omit the field (an explicit `null` also reads as "none";
the saver writes neither).

## `section split` run partitioning

The `section split [section=<idx>] [at=<grapheme>]` console verb
(SECTIONS_BORDERS_RESIZE_PLAN.md §4.5) splits a section in two
at a grapheme boundary; the prefix stays at `idx`, the suffix
becomes a new section at `idx + 1`.

`text_runs` partition grapheme-correctly via
[`text_run_ops::slice`](../lib/baumhard/src/mindmap/model/text_run_ops.rs):
- Runs wholly inside `[0, split_grapheme)` survive unchanged on
  the prefix.
- Runs wholly inside `[split_grapheme, total_graphemes)` survive
  on the suffix with their `start` / `end` shifted by
  `-split_grapheme` into the new section's coordinate space.
- Runs **straddling** the split are clipped: prefix gets the
  in-prefix portion clamped at `split_grapheme`; suffix gets
  the in-suffix portion clamped at `0` (after the
  `-split_grapheme` shift) — both halves carry the same style
  attributes.

The grapheme-correct partitioning means a styled section
round-trips through a split → save → `maptool verify` cycle
without invariant violations. Pre-Batch-5 `split_section`
compared grapheme-indexed run boundaries against byte offsets,
silently corrupting runs on any non-ASCII text.
