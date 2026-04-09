# miMind Format Reference

This document describes the miMind mind map file format, reverse-engineered from
analysis of exported `.mind` files and the Content.xml structure within them.

## File Container

A `.mind` file is a **7-zip archive** containing:

| File | Purpose |
|------|---------|
| `Content.xml` | Main mind map structure, nodes, connections, styling |
| `Header.xml` | Metadata and theme properties |
| `Preview.jpg` | Thumbnail image of the map |
| `ModifiedTime.txt` | Last modification timestamp |
| `id.txt` | Map identifier |
| `Signature.txt` | Signature/version info |

All structural data lives in `Content.xml`.

## XML Quirks

miMind's XML has several non-standard features that require preprocessing:

1. **Numeric attribute names**: `<mpBold 0="1">` uses `0` as an attribute name,
   which is invalid XML. Must be prefixed (e.g., `p0="1"`) before parsing.

2. **Bracket notation**: `vecUserAdjustedMidPts[0]="value"` uses brackets in
   attribute names. Must be converted (e.g., `_0="value"`).

3. **HTML entity for minus**: Negative numbers are encoded as `&#045;` instead
   of a literal `-` character. E.g., `iRelativeX="&#045;179.43"` means -179.43.

## Root Element

```xml
<Content cBackgroundColor="20,20,20,255,1">
  <MindMapNode ...>...</MindMapNode>
  <MindMapNode ...>...</MindMapNode>
</Content>
```

- `cBackgroundColor`: Canvas background in RGBA format (see Color Format below).
- Contains top-level `MindMapNode` elements (root nodes have `iParentID="0"`).

## Color Format

Colors use the format `"R,G,B,A,flag"`:
- R, G, B: 0-255 intensity values
- A: 0-255 alpha (0 = transparent, 255 = opaque)
- flag: 0 or 1 (transparency handling; 0 with A=0 means transparent)

Example: `"169,222,203,255,1"` = RGB(169,222,203), fully opaque.

## MindMapNode

Each node in the mind map is a `<MindMapNode>` element at the top level of
`<Content>`. Hierarchy is expressed via `iParentID` references, not XML nesting.

### Key Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `iNodeID` | string | Unique node identifier |
| `iParentID` | string | Parent node ID (`"0"` = root / no parent) |
| `iIndex` | int | Ordering among siblings (sorted ascending) |
| `iRelativeX` | float | **Absolute** X canvas position (despite the name) |
| `iRelativeY` | float | **Absolute** Y canvas position |
| `iWidth` | float | Node width in pixels |
| `iHeight` | float | Node height in pixels |
| `bSubNodesExpanded` | "0"/"1" | Fold state: "0" = collapsed, "1" = expanded |
| `eSubNodeLayout` | int | Layout algorithm for children (see Layout System) |
| `eSubNodeLayoutDirection` | int | Branch direction for children (see Layout System) |
| `fSubNodeLayoutSpacing` | float | Spacing between children (default: 50) |
| `fFrameThickness` | float | Border thickness |

**Important**: `iRelativeX` and `iRelativeY` are absolute canvas coordinates,
not offsets from the parent node. All nodes share a single coordinate space.

### Child Elements

```xml
<MindMapNode ...>
  <MindMapNodeChild ...>
    <EditText ...>
      <Text>Node text content</Text>
      <TextFormat ...>...</TextFormat>
    </EditText>
  </MindMapNodeChild>
  <MindMapConnection .../>
  <MindMapConnection .../>
  <colorSchema ...>
    <ColorGroup .../>
  </colorSchema>
  <sNotes>Note text</sNotes>
  <TextFormat .../>
  <NodeShape .../>
</MindMapNode>
```

## Layout System

The layout is controlled by two attributes on each node, defining how that
node's **children** are arranged.

### `eSubNodeLayout` (layout algorithm)

| Value | Name | Behavior |
|-------|------|----------|
| 0 | Map | Free/scattered placement. Children are user-positioned on the 2D canvas with no grid or line constraints. |
| 1 | Tree | Structured arrangement. One axis is fixed (branch distance from parent), children spread along the perpendicular axis. |
| 2 | Outline | Linear list. Children placed in a single line at a fixed distance from parent. |

### `eSubNodeLayoutDirection` (branch direction)

| Value | Name | Tree behavior | Map behavior |
|-------|------|---------------|--------------|
| 0 | Auto | - | Free placement, slight rightward+downward bias |
| 1 | Up | Horizontal row above parent (fixed dy, varying dx) | Biased upward |
| 2 | Down | Horizontal row below parent (fixed dy, varying dx) | Biased downward |
| 3 | Left | Vertical column left of parent (fixed dx, varying dy) | Biased leftward |
| 4 | Right | Vertical column right of parent (fixed dx, varying dy) | Biased rightward |
| 6 | Balanced | - | Radial scatter, approximately even left/right split |

**Tree layout specifics**: In tree mode, siblings share a common coordinate
on the branch axis. The perpendicular axis provides the spread:
- Tree + Right: children at same dx (e.g., +407), spread vertically (varying dy)
- Tree + Down: children at same dy (e.g., +176), spread horizontally (varying dx)
- Tree + Left: children at same dx (e.g., -280), spread vertically
- Tree + Up: children at same dy (e.g., -142), spread horizontally

`fSubNodeLayoutSpacing` controls the gap between siblings along the spread axis.

## Text Content and Formatting

### Text Element

Plain text is in `<Text>` inside `<EditText>` inside `<MindMapNodeChild>`:

```xml
<MindMapNodeChild ...>
  <EditText ...>
    <Text>Hello World</Text>
    <TextFormat alignment="1" significance="1" uUserChanges="229">
      <mpBold 0="1" 5="0"/>
      <mpItalic 0="0"/>
      <mpUnderline 0="0"/>
      <mpTextHeight 0="30"/>
      <mpTextColor 0="255,255,255,255,0"/>
      <mpTextFont 0="LiberationSans"/>
      <mpHyperlink 0=""/>
    </TextFormat>
  </EditText>
</MindMapNodeChild>
```

### Position-Based Formatting (mp* Elements)

Text formatting uses a **position-keyed** system. Each `mp*` element has
attributes where the name is a character position and the value is the format
state from that position onward.

Example: `<mpBold 0="1" 5="0">` means:
- Characters 0-4: **bold**
- Characters 5+: not bold

The numeric attribute names (after preprocessing) become `p0`, `p5`, etc.

### Format Channels

| Element | Value type | Description |
|---------|-----------|-------------|
| `mpBold` | "0"/"1" | Bold on/off |
| `mpItalic` | "0"/"1" | Italic on/off |
| `mpUnderline` | "0"/"1" | Underline on/off |
| `mpStrikeOut` | "0"/"1" | Strikethrough on/off |
| `mpTextHeight` | float | Text height in pixels (multiply by 0.75 for points) |
| `mpTextColor` | RGBA string | Text color |
| `mpTextFont` | string | Font family name |
| `mpHyperlink` | string | URL (empty string = no link) |
| `mpSelectionColor` | RGBA string | Selection highlight color |

### TextFormat significance

- `significance="1"`: Node content text formatting
- `significance="3"`: Connection label text formatting

## NodeShape

Defines the visual appearance of the node box:

```xml
<NodeShape
  type="5"
  cBackColor="20,20,20,255,1"
  cFrameColor="48,176,130,255,0"
  bDoFrame="0"
  bDoShadow="0"
  fCornerRadiusPercent="10"
  fCornerRadiusLength="25"
  fFrameThickness="4"
  ptSize="373.515625,109"/>
```

| Attribute | Type | Description |
|-----------|------|-------------|
| `type` | int | Shape type identifier |
| `cBackColor` | RGBA | Background fill color |
| `cFrameColor` | RGBA | Border color |
| `bDoFrame` | "0"/"1" | Whether to draw the border |
| `bDoShadow` | "0"/"1" | Whether to draw a drop shadow |
| `fCornerRadiusPercent` | float | Corner rounding as percentage |
| `fCornerRadiusLength` | float | Corner rounding as absolute length |
| `fFrameThickness` | float | Border width |
| `ptSize` | "w,h" | Dimensions (often redundant with iWidth/iHeight) |

## Color Schema System

Color schemas are hierarchical themes applied to entire subtrees.

### Structure

```xml
<colorSchema
  paletteName="coral"
  type="3"
  level="0"
  con="1"
  startsAtRoot="1"
  cThemeBackColors="Pastel:#BFFFFFFE01">
  <ColorGroup cColorBackground="169,222,203,255,1"
              cColorFrame="48,176,130,255,1"
              cColorText="0,0,0,255,1"
              cColorTitle="0,0,0,255,1"/>
  <ColorGroup cColorBackground="243,177,196,255,1"
              cColorFrame="226,66,113,255,1"
              cColorText="0,0,0,255,1"
              cColorTitle="0,0,0,255,1"/>
  <!-- More ColorGroups for deeper levels -->
</colorSchema>
```

### How It Works

- **Schema root node**: Has `level="0"` and contains `ColorGroup` children
  that define colors for each depth level in the subtree.
- **Inheriting nodes**: Deeper nodes have the same `paletteName` but
  `level="1"`, `level="2"`, etc. They do NOT repeat the ColorGroup list;
  they inherit from their schema root.
- **ColorGroup indexing**: `[0]` = colors for the schema root, `[1]` = its
  direct children, `[2]` = grandchildren, and so on.

### colorSchema Attributes

| Attribute | Description |
|-----------|-------------|
| `paletteName` | Theme name (e.g., "coral", "sweet", "sandy", "mountain", "rocky", "rainbow", "autumn", "summer", "navy", "stars", or custom names) |
| `type` | Palette variant (2 = standard, 3 = alternate) |
| `level` | Node's depth from schema root (0 = root) |
| `con` | "1" = connections inherit colors from schema |
| `startsAtRoot` | "1" = level counting starts from schema root |
| `cThemeBackColors` | Theme identifier string (e.g., "Pastel:#BFFFFFFE01") |

### ColorGroup Attributes

| Attribute | Description |
|-----------|-------------|
| `cColorBackground` | Node background fill (RGBA) |
| `cColorFrame` | Node border/frame color (RGBA) |
| `cColorText` | Body text color (RGBA) |
| `cColorTitle` | Title text color (RGBA) |

## MindMapConnection

Connections are child elements of their **origin** node's `<MindMapNode>`.
They represent both parent-child edges and cross-links.

```xml
<MindMapConnection
  iOriginNodeID="348068464"
  iTargetNodeID="351582192"
  bIsCrossLink="0"
  bConnectionVisible="1"
  cColor="48,176,130,255,0"
  fLineWidth="6"
  style="0"
  type="0"
  eShapeOrigin="0"
  eShapeTarget="3"
  vecUserAdjustedMidPts[0]="&#045;597.612244,112.673889"
  vecUserAdjustedMidPts[1]="276.831512,&#045;49.547791"
  ptUserAdjustedTarget="9.824127,411.440369">
  <!-- Optional label -->
  <EditText ...>
    <Text>Label text</Text>
    <TextFormat significance="3" .../>
  </EditText>
</MindMapConnection>
```

### Connection Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `iOriginNodeID` | string | Source node ID |
| `iTargetNodeID` | string | Target node ID |
| `bIsCrossLink` | "0"/"1" | "0" = parent-child edge, "1" = cross-link |
| `bConnectionVisible` | "0"/"1" | Whether the connection is drawn |
| `cColor` | RGBA | Line color |
| `fLineWidth` | float | Line thickness |
| `style` | int | Line style (0 = default, 1 = alternative) |
| `type` | int | Connection type variant |
| `eShapeOrigin` | int | Anchor point on origin node (0 = auto, 3 = specific side) |
| `eShapeTarget` | int | Anchor point on target node |

### Control Points

Curved connections can have user-adjusted midpoints:

- `vecUserAdjustedMidPts[0]` through `[N]`: Bezier control points as `"x,y"` pairs
- `ptUserAdjustedTarget`: Custom target attachment point as `"x,y"`

### Connection Labels

Labels are `<EditText>` children with `<TextFormat significance="3">`.
The text content is in the nested `<Text>` element.

## Notes

Node notes are stored in `<sNotes>` elements:

```xml
<sNotes>Note content here</sNotes>
```

Empty string if no notes.
