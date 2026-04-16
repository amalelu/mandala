# Text Runs

`text_runs` is a list of formatting spans applied to a node's `text`.

```json
{
  "text": "Hello world",
  "text_runs": [
    { "start": 0, "end": 5, "bold": true, "font": "LiberationSans", "size_pt": 14, "color": "#ffffff" }
  ]
}
```

Each run covers a character range `[start, end)` and carries formatting
metadata: `bold`, `italic`, `underline`, `font`, `size_pt`, `color`, and
optional `hyperlink`.

## Why runs are optional

`text_runs` defaults to an empty array when absent. A node with empty runs
renders its entire text using the node's base style (`style.text_color`,
a default font, a default size).

This matters for hand-authoring. A simple node becomes:

```json
"0": { "id": "0", "text": "Hello", "style": { "text_color": "#ffffff", ... }, ... }
```

No noise. No need to declare a run that just repeats the style.

## Coverage rules

When `text_runs` is non-empty:

- Runs must be ordered by ascending `start`
- Runs must not overlap: each run's `end <= next.start`
- `start < end` for every run
- `start` and `end` are measured in **Unicode code points** (Rust's
  `char` count) — not bytes, not grapheme clusters
- Uncovered ranges inherit the node's base style (so partial coverage is
  valid — you can decorate just the first word without declaring runs for
  the rest)

Code points match how `ColorFontRegions` interprets the ranges in the
tree builder and how miMind stored them in the legacy format. Hebrew text
with combining marks has more code points than graphemes; CJK characters
outside the BMP have more bytes than code points. Code points are the
unit that stays stable through round-trips.

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

"Hello" is bold. " world" inherits the node's base `style.text_color`,
default font, default size. Valid.

## Hyperlinks

A run can set `"hyperlink": "https://example.com"`. The renderer draws
the covered text as a clickable link styled with that URL. Runs without a
hyperlink set the field to `null` (or omit it — it's serde-optional).
