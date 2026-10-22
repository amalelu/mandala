// SPDX-License-Identifier: MPL-2.0

//! Per-node border rendering vocabulary — the `GlyphBorder*` config
//! structs the loader deserializes and the geometry constants the
//! renderer and `tree_builder::build_border_tree` share to keep
//! border layout consistent across the two paths. Borders are the
//! glyph-drawn rectangles around framed nodes; portal labels, edge
//! handles, and drag previews all attach to these geometry hints.

use serde::{Deserialize, Serialize};

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
// `FontSystem` is re-exported by `crate::font` (§B5: code outside
// `font/` does not name `cosmic_text` directly). Threaded through
// `border_run_specs_with` so guard-holding callers measure without a
// nested `FONT_SYSTEM` acquire.
use crate::font::FontSystem;
use crate::mindmap::border_pattern::SidePattern;
use crate::mindmap::model::{
    default_border_font_size, Canvas, ColorGroup, CustomBorderGlyphs, GlyphBorderConfig, MindSection,
};
use crate::util::color::FloatRgba;
use crate::util::grapheme_chad::{count_grapheme_clusters, join_graphemes};

/// Fraction of `font_size` by which a border's top/bottom runs
/// are pulled inward so their glyph visible extents overlap with
/// the vertical columns. Empirically chosen for LiberationSans at
/// typical border font sizes. Shared by the renderer and
/// `tree_builder::build_border_tree` so the two paths can't drift.
/// Per-face calibration uses
/// [`crate::font::fonts::measure_glyph_ink_bounds`] when the chosen
/// border face differs from LiberationSans.
pub const BORDER_CORNER_OVERLAP_FRAC: f32 = 0.35;

/// Multiplier estimating one border-glyph advance as a fraction of
/// `font_size`. `0.6` matches LiberationSans box-drawing
/// characters; both the renderer's keyed border-buffer rebuild and
/// the border-tree builder consult this for corner positioning.
/// Per-face calibration measures the actual `─` advance via
/// cosmic-text shaping when the border face differs.
pub const BORDER_APPROX_CHAR_WIDTH_FRAC: f32 = 0.6;

/// Defines which glyphs to use for rendering a node's border.
/// Each field is a single character (glyph) from the selected font.
/// The border is rendered as positioned text elements around the node content.
#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct BorderGlyphSet {
    /// Horizontal fill glyph used along the top edge between corners.
    pub top: char,
    /// Horizontal fill glyph used along the bottom edge between corners.
    pub bottom: char,
    /// Vertical fill glyph used along the left edge between corners.
    pub left: char,
    /// Vertical fill glyph used along the right edge between corners.
    pub right: char,
    /// Single-character glyph in the top-left corner.
    pub top_left: char,
    /// Single-character glyph in the top-right corner.
    pub top_right: char,
    /// Single-character glyph in the bottom-left corner.
    pub bottom_left: char,
    /// Single-character glyph in the bottom-right corner.
    pub bottom_right: char,
}

/// Single source of truth for the four canonical Unicode
/// box-drawing presets. Each row is `(name, [top, bottom, left,
/// right, tl, tr, bl, br], hint)`, where `hint` is the one-line
/// human description [`border_preset_hint`] serves to the console's
/// completion popup. Adding a fifth preset is a one-row extension
/// here — [`BORDER_PRESETS`] and the hint lookup both derive from
/// this table, and the tuple's third slot means a new preset cannot
/// land without a description.
const PRESET_TABLE: &[(&str, [char; 8], &str)] = &[
    (
        "light",
        ['─', '─', '│', '│', '┌', '┐', '└', '┘'],
        "thin lines (default)",
    ),
    ("heavy", ['━', '━', '┃', '┃', '┏', '┓', '┗', '┛'], "bold lines"),
    ("double", ['═', '═', '║', '║', '╔', '╗', '╚', '╝'], "double lines"),
    (
        "rounded",
        ['─', '─', '│', '│', '╭', '╮', '╰', '╯'],
        "thin lines with rounded corners",
    ),
];

impl BorderGlyphSet {
    /// Build a glyph set from the 8-char layout `[top, bottom,
    /// left, right, tl, tr, bl, br]` used by [`PRESET_TABLE`].
    fn from_glyphs(g: [char; 8]) -> Self {
        BorderGlyphSet {
            top: g[0],
            bottom: g[1],
            left: g[2],
            right: g[3],
            top_left: g[4],
            top_right: g[5],
            bottom_left: g[6],
            bottom_right: g[7],
        }
    }

    /// Standard Unicode box-drawing characters (light lines).
    pub fn box_drawing_light() -> Self {
        Self::from_glyphs(PRESET_TABLE[0].1)
    }

    /// Heavy box-drawing characters.
    pub fn box_drawing_heavy() -> Self {
        Self::from_glyphs(PRESET_TABLE[1].1)
    }

    /// Double-line box-drawing characters.
    pub fn box_drawing_double() -> Self {
        Self::from_glyphs(PRESET_TABLE[2].1)
    }

    /// Rounded box-drawing characters.
    pub fn box_drawing_rounded() -> Self {
        Self::from_glyphs(PRESET_TABLE[3].1)
    }

    /// Generates the top border string for a given width in characters.
    pub fn top_border(&self, char_width: usize) -> String {
        if char_width < 2 {
            return String::new();
        }
        let mut s = String::with_capacity(char_width);
        s.push(self.top_left);
        for _ in 0..char_width.saturating_sub(2) {
            s.push(self.top);
        }
        s.push(self.top_right);
        s
    }

    /// Generates the bottom border string for a given width in characters.
    pub fn bottom_border(&self, char_width: usize) -> String {
        if char_width < 2 {
            return String::new();
        }
        let mut s = String::with_capacity(char_width);
        s.push(self.bottom_left);
        for _ in 0..char_width.saturating_sub(2) {
            s.push(self.bottom);
        }
        s.push(self.bottom_right);
        s
    }

    /// Generates a left side character (repeated for each row).
    pub fn left_char(&self) -> char {
        self.left
    }

    /// Generates a right side character (repeated for each row).
    pub fn right_char(&self) -> char {
        self.right
    }

    /// Generates a vertical side column of `rows` rows, using
    /// `self.left` as the glyph. Rows are separated by `'\n'`, and
    /// the returned string ends without a trailing newline — one
    /// glyph per line cell, `rows` lines total.
    ///
    /// Callers that want the right side can either use this same
    /// string (since the rounded/light presets have `left == right`)
    /// or call `right_side_border` below for an explicit right column.
    ///
    /// Cost: O(rows) push operations, one allocation sized to
    /// `rows * (left.len_utf8() + 1)`.
    pub fn side_border(&self, rows: usize) -> String {
        build_side_column(self.left, rows)
    }

    /// Like [`Self::side_border`] but uses `self.right`. Presets where
    /// `left == right` can call either — this exists so callers
    /// never need to know which preset they have.
    pub fn right_side_border(&self, rows: usize) -> String {
        build_side_column(self.right, rows)
    }
}

fn build_side_column(glyph: char, rows: usize) -> String {
    if rows == 0 {
        return String::new();
    }
    let glyph_len = glyph.len_utf8();
    let mut s = String::with_capacity(rows * (glyph_len + 1) - 1);
    for i in 0..rows {
        s.push(glyph);
        if i + 1 < rows {
            s.push('\n');
        }
    }
    s
}

/// Runtime form of the four border corners as
/// grapheme-cluster-ready strings. The data model
/// ([`CustomBorderGlyphs`]) already stores corners as `String`, so
/// a user-supplied multi-cluster corner like `tl = "<<"` survives
/// serialization round-trip; this type is its render-time shape,
/// populated by [`resolve_border_style`] from either a
/// `CustomBorderGlyphs` payload (when the preset is `"custom"`)
/// or the chosen preset's single-char defaults.
///
/// Each field is one or more grapheme clusters. Empty strings are
/// allowed by the type but actively normalized to the preset
/// fallback during resolution, so the renderer never receives a
/// corner with zero glyphs.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct BorderCorners {
    /// Glyphs in the top-left corner of the rectangle.
    pub top_left: String,
    /// Glyphs in the top-right corner of the rectangle.
    pub top_right: String,
    /// Glyphs in the bottom-left corner of the rectangle.
    pub bottom_left: String,
    /// Glyphs in the bottom-right corner of the rectangle.
    pub bottom_right: String,
}

/// Parsed [`SidePattern`] for each of the four sides — what the
/// renderer fits between the corners. Populated by
/// [`resolve_border_style`] from the per-node config or the
/// preset's defaults.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct SidePatternQuad {
    /// Pattern fitted between the top corners.
    pub top: SidePattern,
    /// Pattern fitted between the bottom corners.
    pub bottom: SidePattern,
    /// Pattern repeated down the left column.
    pub left: SidePattern,
    /// Pattern repeated down the right column.
    pub right: SidePattern,
}

/// Which `ColorGroup` channel the border cycles through when a
/// `color_palette` is bound. Defaults to [`PaletteField::Frame`]
/// because frame is the channel whose meaning matches the border
/// today — the same channel
/// [`MindMap::node_frame_color`](crate::mindmap::model::MindMap::node_frame_color)
/// hands the single-color path as its cascade base. Open seam —
/// adding a new variant here is a localized change.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum PaletteField {
    /// Cycle the border across each `ColorGroup`'s `frame`
    /// channel — the historical default and the channel whose
    /// meaning matches the border itself.
    //#[default]
    Frame,
    /// Cycle across each group's `background` channel — useful
    /// when the border should track the node fill rather than the
    /// frame stroke.
    Background,
    /// Cycle across each group's `text` channel — for borders
    /// drawn in the same hue as the node label.
    Text,
    /// Cycle across each group's `title` channel — for borders
    /// drawn in the same hue as the node title.
    Title,
}

impl PaletteField {
    /// Parse the `color_palette_field` string from the data model.
    /// Unknown values warn and fall back to `Frame` so a typo
    /// degrades to "single color" instead of dropping the whole
    /// border per `CODE_CONVENTIONS.md` §9.
    pub fn from_str_or_default(s: Option<&str>) -> Self {
        match s.map(str::to_ascii_lowercase).as_deref() {
            Some("frame") | None => PaletteField::Frame,
            Some("background") => PaletteField::Background,
            Some("text") => PaletteField::Text,
            Some("title") => PaletteField::Title,
            Some(other) => {
                log::warn!(
                    "mindmap::border: color_palette_field '{}' unknown; using 'frame'",
                    other
                );
                PaletteField::Frame
            }
        }
    }

    /// One-word readable label (`"frame"`, `"background"`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            PaletteField::Frame => "frame",
            PaletteField::Background => "background",
            PaletteField::Text => "text",
            PaletteField::Title => "title",
        }
    }

    /// Read the channel value out of one [`ColorGroup`].
    pub fn read(self, group: &ColorGroup) -> &str {
        match self {
            PaletteField::Frame => &group.frame,
            PaletteField::Background => &group.background,
            PaletteField::Text => &group.text,
            PaletteField::Title => &group.title,
        }
    }

    /// Static list of recognized values (used by the console
    /// command's completion).
    pub const ALL: &'static [&'static str] = &["frame", "background", "text", "title"];
}

/// Configuration for how a node's border should be rendered.
/// This struct is intended to be attached per-node or as a global default,
/// and is the key extensibility point for the editing experience.
#[derive(Debug, Clone)]
pub struct BorderStyle {
    /// Legacy single-character glyph set — kept on the type so
    /// callers that only need the simple box-drawing presets
    /// (e.g. the console overlay frame in
    /// `src/application/renderer/console_geometry.rs`) can keep
    /// using `top_border` / `side_border` etc. unchanged.
    pub glyph_set: BorderGlyphSet,
    /// Multi-cluster runtime corners. Populated by
    /// [`resolve_border_style`]; defaults to the light preset's
    /// corners (`┌┐└┘`) as single-cluster strings.
    pub corners: BorderCorners,
    /// Parsed [`SidePattern`] for each side. Populated by
    /// [`resolve_border_style`] from the user's
    /// `CustomBorderGlyphs` input or from preset defaults.
    pub side_patterns: SidePatternQuad,
    /// Optional palette name to cycle per-glyph colors from.
    /// Renderer-time lookup happens in [`build_border_regions`].
    pub color_palette: Option<String>,
    /// Which channel of each `ColorGroup` is cycled when
    /// `color_palette` is bound.
    pub palette_field: PaletteField,
    /// The font to use for border glyphs. None means use default system font.
    pub font_name: Option<String>,
    /// Font size for border glyphs in points.
    pub font_size_pt: f32,
    /// Border color as #RRGGBB hex string. Used as the fallback
    /// when `color_palette` is unset or fails to resolve.
    pub color: String,
    /// Whether to render this border at all.
    pub visible: bool,
}

impl BorderStyle {
    /// Construct a default visible border with the given color
    /// (`#RRGGBB` hex or resolved theme variable). Uses the light
    /// box-drawing preset + 14 pt default font size — matches the
    /// scene builder's per-framed-node default so node borders keep
    /// the same look when a caller asks for "a default border in this
    /// color" instead of building one field-by-field.
    ///
    /// `light` (`┌─│┘`) is the chosen default rather than `rounded`
    /// (`╭─│╯`) because the rounded corners curve inward away from
    /// the cell edges, leaving a visible gap where corner meets
    /// side in any monospace face. The light preset's corners
    /// extend to the cell edges, so corner and side connect cleanly.
    pub fn default_with_color(color: &str) -> Self {
        let glyph_set = BorderGlyphSet::box_drawing_light();
        BorderStyle {
            corners: glyph_set.corners(),
            side_patterns: glyph_set.side_patterns(),
            glyph_set,
            color_palette: None,
            palette_field: PaletteField::Frame,
            font_name: None,
            font_size_pt: default_border_font_size(),
            color: color.to_string(),
            visible: true,
        }
    }

    /// Vertical column for the left side at `rows` rows. Each
    /// rendered cluster occupies one line; clusters are separated
    /// by `'\n'` and the last cluster has no trailing newline,
    /// mirroring the legacy `BorderGlyphSet::side_border` shape.
    pub fn left_column_text(&self, rows: usize) -> String {
        build_vertical_text(&self.side_patterns.left, rows)
    }

    /// Vertical column for the right side at `rows` rows.
    pub fn right_column_text(&self, rows: usize) -> String {
        build_vertical_text(&self.side_patterns.right, rows)
    }

    /// Hash every resolved axis of this style into `state`.
    ///
    /// The one place a border's style is folded into a canvas-role
    /// signature. Both consumers — `border_content_signature` and
    /// `section_frame_content_signature` in
    /// [`crate::mindmap::tree_builder`] — reach for this rather
    /// than listing fields themselves, so a tenth axis on this
    /// struct cannot reach [`border_run_specs`] while one of the
    /// two signatures stays blind to it.
    ///
    /// Deliberately over-inclusive: `glyph_set` and `visible` reach
    /// no glyph run that either signature guards, but hashing a
    /// field the projection ignores can only cost a redundant
    /// rebuild, while omitting one the projection reads serves a
    /// stale frame. `font_size_pt` goes in as `to_bits`.
    ///
    /// Cost: O(total corner + pattern bytes); no allocation. The
    /// body destructures `self`, so a field added to this struct
    /// will not compile until it is accounted for here.
    pub fn hash_content<H: std::hash::Hasher>(&self, state: &mut H) {
        use std::hash::Hash;
        let BorderStyle {
            glyph_set,
            corners,
            side_patterns,
            color_palette,
            palette_field,
            font_name,
            font_size_pt,
            color,
            visible,
        } = self;
        glyph_set.hash(state);
        corners.hash(state);
        side_patterns.hash(state);
        color_palette.hash(state);
        palette_field.hash(state);
        font_name.hash(state);
        font_size_pt.to_bits().hash(state);
        color.hash(state);
        visible.hash(state);
    }

    /// Cluster count of each corner — handed to the fitter and
    /// the auto-resize pass so they speak in the same units.
    pub fn corner_clusters(&self) -> CornerClusterCounts {
        CornerClusterCounts {
            top_left: count_grapheme_clusters(&self.corners.top_left),
            top_right: count_grapheme_clusters(&self.corners.top_right),
            bottom_left: count_grapheme_clusters(&self.corners.bottom_left),
            bottom_right: count_grapheme_clusters(&self.corners.bottom_right),
        }
    }
}

/// Per-side run geometry the three border-emit paths (the
/// in-place mutator path, the initial-build tree path, and the
/// section-frame tree path) each previously open-coded with
/// byte-identical math.
///
/// One spec describes one side (top / bottom / left / right):
/// where the run sits in canvas space, how big its text bounds
/// are, what glyph string it carries, what palette offset to
/// hand to [`build_border_regions`], and the pre-counted
/// grapheme cluster count so consumers don't re-walk the string.
///
/// Pure data — no allocation beyond the `String` text. Consumers
/// translate the spec into their pipeline-specific output:
/// the tree path wraps it into a [`crate::gfx_structs::area::GlyphArea`];
/// the renderer's flat path shapes it into a `cosmic_text::Buffer`.
/// Color, palette cycle, and zoom-visibility belong with the
/// consumer (those are policy, not geometry).
#[derive(Clone, Debug, PartialEq)]
pub struct BorderRunSpec {
    /// 1=top-fill, 2=bottom-fill, 3=left-fill, 4=right-fill,
    /// 5=TL corner, 6=TR corner, 7=BL corner, 8=BR corner.
    /// Stable across rebuilds — the in-place mutator path keys
    /// leaves on this channel.
    pub channel: usize,
    /// Glyph string for this run. For rails this is the fill
    /// pattern (no corners). For corner specs this is the
    /// single corner glyph (a single grapheme cluster).
    pub text: String,
    /// Font size in pt; identical for all specs (sourced from
    /// the [`BorderStyle::font_size_pt`]).
    pub font_size_pt: f32,
    /// Line-height (y-stride between cluster rows) used by the
    /// renderer's buffer for this spec. Defaults to
    /// `font_size_pt` (cosmic-text's default). For vertical
    /// rails we set this to the **measured ink height** of the
    /// fill glyph so consecutive cluster rows touch — without
    /// this override, the renderer stacks each row at
    /// `line_height = font_size` of vertical space, leaving an
    /// `(font_size − ink_height)` empty gap between rows that
    /// reads as "gappy diamonds" on filled-glyph patterns.
    pub line_height_pt: f32,
    /// Top-left position of the run's text bounds in canvas space.
    pub position: (f32, f32),
    /// Width / height of the run's text bounds.
    pub bounds: (f32, f32),
    /// Glyph-index offset into the per-cycle palette so a palette-cycling
    /// border sweeps continuously around the rectangle in
    /// top → right → bottom → left order. Zero when the upstream
    /// palette is empty (single-color border).
    pub palette_offset: usize,
    /// Pre-computed `count_grapheme_clusters(text)`. Carried on
    /// the spec so consumers handing it to [`build_border_regions`]
    /// don't re-walk the string.
    pub cluster_count: usize,
}

/// Compute the run geometry for one node's border — eight specs:
/// four fill rails and four corners.
/// Single source of truth for the per-side `(text, position,
/// bounds, palette_offset)` arithmetic that the in-place mutator
/// path, the initial-build tree path, and the section-frame tree
/// path previously reproduced independently.
///
/// This is the **lock-acquiring wrapper** for callers that do NOT
/// already hold the `FONT_SYSTEM` write guard (the tree-builder
/// paths, unit tests). A caller inside a write-guard scope must use
/// [`border_run_specs_with`] instead, or the nested same-thread
/// acquire deadlocks (issue P0-06).
///
/// Channels — the contract the in-place mutator path keys its
/// leaves on, and the one [`BorderRunSpec::channel`] states
/// field-side:
/// - `1` = top fill, `2` = bottom fill, `3` = left fill,
///   `4` = right fill,
/// - `5` = top-left corner, `6` = top-right, `7` = bottom-left,
///   `8` = bottom-right.
///
/// The count is fixed at eight for every input: each spec is
/// pushed unconditionally, an empty side yielding an empty run
/// rather than no run. Callers rely on that — `tree_builder`'s
/// structural signature covers the framed-node sequence and *not*
/// a per-node run count, on the grounds that there is no input
/// that can vary one.
///
/// Palette offsets place each run in one clockwise sweep starting
/// at the top-left corner — TL, top fill, TR, right fill, BR,
/// bottom fill, BL, left fill — each entry advancing by the
/// cluster count of the run before it. The arithmetic is in the
/// body rather than restated here, because a formula copied into
/// prose is one that goes stale silently: the four-term version
/// this paragraph used to carry predated the corners and was
/// wrong by four terms. Vertical text strings include `'\n'`
/// separators which the grapheme counter folds into one cluster
/// per visible glyph, so the indices line up with the per-cluster
/// regions [`build_border_regions`] emits.
///
/// Cost: acquires the `FONT_SYSTEM` **write** guard once per call —
/// unconditionally, even when every glyph is a cache hit (unlike the
/// metric-cache wrappers, which check the cache before locking).
/// Shapes each corner + fill grapheme once through the metric cache
/// (hot re-renders hit the cache; only cold keys touch cosmic-text),
/// 4 `String` allocations, 4 `count_grapheme_clusters` walks. Same
/// inputs → same array.
pub fn border_run_specs(
    border_style: &BorderStyle,
    node_pos: (f32, f32),
    node_size: (f32, f32),
) -> Vec<BorderRunSpec> {
    // Warm the font lazy-statics BEFORE taking the guard. Resolving a
    // face under the guard lazily builds `FAMILY_INDEX` /
    // `COMPILED_FONT_ID_MAP`, and that build acquires `FONT_SYSTEM`
    // via `load_fonts` — a same-thread re-entry that would time out.
    // Production warms this at `fonts::init()`, so this is a no-op
    // there; it only matters for unlocked callers that skip `init()`
    // (the tree-builder path, unit tests). See `fonts::ensure_warm`.
    crate::font::fonts::ensure_warm();
    let mut font_system = crate::font::fonts::acquire_font_system_write("border_run_specs");
    border_run_specs_with(&mut font_system, border_style, node_pos, node_size)
}

/// [`border_run_specs`] for callers that already hold the
/// `FONT_SYSTEM` write guard. Threads the guard into every
/// metric-cache measurement so a cold corner / fill grapheme
/// shapes through the held guard instead
/// of blocking on a second acquire of the same lock — the composable
/// design §B5 and the `measure_glyph_ink_bounds` primitive share.
///
/// Assumes `fonts::init()` has run (the standard §B5 invariant): the
/// face-resolution and pin lookups here read the already-built
/// `FAMILY_INDEX` / `COMPILED_FONT_ID_MAP` without triggering
/// `load_fonts`, so nothing under this guard re-acquires it.
pub fn border_run_specs_with(
    font_system: &mut FontSystem,
    border_style: &BorderStyle,
    node_pos: (f32, f32),
    node_size: (f32, f32),
) -> Vec<BorderRunSpec> {
    use crate::font::fonts::app_font_by_family;
    use crate::font::metric_cache::glyph_ink_with;

    let font_size = border_style.font_size_pt;
    let face = border_style.font_name.as_deref().and_then(app_font_by_family);

    // single-glyph buffer at the exact node corner pixel; the
    // fill rails span the gap BETWEEN corners. Pre-fix the
    // corners were concatenated into the rail text, so cosmic-text
    // packed them at the natural advance after the fill —
    // never landing on the node's actual corner pixel.
    // Per-corner positioning makes corner placement
    // structurally exact.
    let tl_ink = glyph_ink_with(font_system, face, font_size, &border_style.corners.top_left);
    let tr_ink = glyph_ink_with(font_system, face, font_size, &border_style.corners.top_right);
    let bl_ink = glyph_ink_with(font_system, face, font_size, &border_style.corners.bottom_left);
    let br_ink = glyph_ink_with(font_system, face, font_size, &border_style.corners.bottom_right);

    // Top fill rail spans the horizontal gap between TL and TR
    // corners. Its position.x is `node.x + tl_w`; its bounds.0
    // is `node.width − tl_w − tr_w`. cosmic-text packs the fill
    // glyphs starting at position.x; the leftover sub-grapheme
    // gap at the right end is bounded by the smallest fill
    // grapheme width (≤ ~10 px on typical fonts).
    let top_fill_avail = (node_size.0 - tl_ink.advance - tr_ink.advance).max(0.0);
    let bottom_fill_avail = (node_size.0 - bl_ink.advance - br_ink.advance).max(0.0);

    let (top_fill_text, top_fill_clusters, _top_fill_w) = fit_pattern_to_width(
        font_system,
        &border_style.side_patterns.top,
        top_fill_avail,
        face,
        font_size,
    );
    let (bottom_fill_text, bottom_fill_clusters, _bottom_fill_w) = fit_pattern_to_width(
        font_system,
        &border_style.side_patterns.bottom,
        bottom_fill_avail,
        face,
        font_size,
    );

    // Vertical rails: line_height = MEASURED INK HEIGHT of the
    // first fill grapheme. This makes consecutive cluster rows
    // touch, eliminating the gappy-diamonds defect on Multi-cluster
    // (where the `◆` glyph is ~12 pt of ink in an 18 pt
    // line-height box, leaving ~6 pt of empty space between
    // diamonds).
    let left_first_glyph = side_pattern_first_grapheme(&border_style.side_patterns.left);
    let right_first_glyph = side_pattern_first_grapheme(&border_style.side_patterns.right);
    let left_line_h = if !left_first_glyph.is_empty() {
        glyph_ink_with(font_system, face, font_size, &left_first_glyph).ink_height
    } else {
        font_size
    };
    let right_line_h = if !right_first_glyph.is_empty() {
        glyph_ink_with(font_system, face, font_size, &right_first_glyph).ink_height
    } else {
        font_size
    };

    // Side rails span the vertical gap between top and bottom
    // corners — i.e. corner ink-height of TL/TR at top,
    // BL/BR at bottom. Use the MAX of left/right corner ink
    // heights for each end so both side rails start at the
    // same y.
    let top_corner_h = tl_ink.ink_height.max(tr_ink.ink_height);
    let bottom_corner_h = bl_ink.ink_height.max(br_ink.ink_height);
    let side_avail = (node_size.1 - top_corner_h - bottom_corner_h).max(0.0);
    // Same clamp as the horizontal fill, and for the same reason:
    // both terms are authored, and the quotient sizes a `String`.
    // One grapheme per row, but its *byte* cost is whatever the
    // author's pattern spells — a ZWJ sequence is one cluster and
    // dozens of bytes — so the byte ceiling gets the real number
    // rather than the row count.
    //
    // That number is the pattern's per-row mean, not its first
    // grapheme: the rail cycles through the whole cluster vector
    // (`cluster[i % unit]`), so a pattern whose first cluster is one
    // byte and whose second is a thousand would otherwise set the
    // ceiling from the byte and then emit the thousand.
    // `+ VERTICAL_ROW_SEPARATOR_BYTES`: the rail is joined with
    // newlines after rendering, so each row costs its cluster plus
    // the separator that follows it.
    let left_row_count = fill_copies(
        side_avail,
        left_line_h,
        1,
        side_pattern_bytes_per_row(&border_style.side_patterns.left) + VERTICAL_ROW_SEPARATOR_BYTES,
        side_pattern_fixed_bytes(&border_style.side_patterns.left),
    );
    let right_row_count = fill_copies(
        side_avail,
        right_line_h,
        1,
        side_pattern_bytes_per_row(&border_style.side_patterns.right) + VERTICAL_ROW_SEPARATOR_BYTES,
        side_pattern_fixed_bytes(&border_style.side_patterns.right),
    );
    let left_text = border_style.left_column_text(left_row_count.max(1));
    let right_text = border_style.right_column_text(right_row_count.max(1));
    let left_v_height = left_row_count as f32 * left_line_h;
    let right_v_height = right_row_count as f32 * right_line_h;

    let left_v_width =
        side_pattern_max_advance(font_system, &border_style.side_patterns.left, face, font_size) + 1.0;
    let right_v_width =
        side_pattern_max_advance(font_system, &border_style.side_patterns.right, face, font_size) + 1.0;

    // Corner buffer y-position: we want the corner's ink-top
    // to align with the node's top edge. cosmic-text places
    // the glyph at `position.y + ascender` baseline; ink-top
    // = baseline + ink_top (where ink_top is negative for
    // above-baseline ink). So `position.y` such that
    // `position.y + ascender + ink_top = node.y` →
    // `position.y = node.y - ascender - ink_top`. We don't
    // have a separate ascender measure here; approximate as
    // `font_size` (which matches cosmic-text's default
    // line-height treatment).
    let top_corner_y = node_pos.1 - tl_ink.ink_top - font_size * 0.8;
    let bottom_corner_y = node_pos.1 + node_size.1 - bl_ink.ink_height - bl_ink.ink_top - font_size * 0.8;

    // Cluster counts for palette-offset sweep (top → right
    // → bottom → left clockwise).
    let top_clusters = top_fill_clusters;
    let bottom_clusters = bottom_fill_clusters;
    let left_clusters = count_grapheme_clusters(&left_text);
    let right_clusters = count_grapheme_clusters(&right_text);

    // Side rail y-position: start just below the top corner's
    // ink-bottom (which is `node.y + top_corner_h`).
    let side_top_y = node_pos.1 + top_corner_h;

    let mut specs: Vec<BorderRunSpec> = Vec::with_capacity(8);
    // Channel 1: top fill rail.
    let top_fill_clusters_n = count_grapheme_clusters(&top_fill_text);
    specs.push(BorderRunSpec {
        channel: 1,
        text: top_fill_text,
        font_size_pt: font_size,
        line_height_pt: font_size,
        position: (node_pos.0 + tl_ink.advance, top_corner_y),
        bounds: (top_fill_avail, font_size * 1.5),
        palette_offset: 1, // after TL corner
        cluster_count: top_fill_clusters_n,
    });
    // Channel 2: bottom fill rail.
    let bottom_fill_clusters_n = count_grapheme_clusters(&bottom_fill_text);
    specs.push(BorderRunSpec {
        channel: 2,
        text: bottom_fill_text,
        font_size_pt: font_size,
        line_height_pt: font_size,
        position: (node_pos.0 + bl_ink.advance, bottom_corner_y),
        bounds: (bottom_fill_avail, font_size * 1.5),
        // bottom fill rides after top sweep + right sweep + BL.
        palette_offset: 1 + top_clusters + 1 + right_clusters + 1,
        cluster_count: bottom_fill_clusters_n,
    });
    // Channel 3: left fill rail.
    specs.push(BorderRunSpec {
        channel: 3,
        text: left_text,
        font_size_pt: font_size,
        line_height_pt: left_line_h,
        position: (node_pos.0, side_top_y),
        bounds: (left_v_width, left_v_height.max(left_line_h)),
        palette_offset: 1 + top_clusters + 1 + right_clusters + 1 + bottom_clusters + 1,
        cluster_count: left_clusters,
    });
    // Channel 4: right fill rail.
    specs.push(BorderRunSpec {
        channel: 4,
        text: right_text,
        font_size_pt: font_size,
        line_height_pt: right_line_h,
        position: (node_pos.0 + node_size.0 - right_v_width, side_top_y),
        bounds: (right_v_width, right_v_height.max(right_line_h)),
        palette_offset: 1 + top_clusters + 1,
        cluster_count: right_clusters,
    });
    // Channel 5: TL corner.
    specs.push(BorderRunSpec {
        channel: 5,
        text: border_style.corners.top_left.clone(),
        font_size_pt: font_size,
        line_height_pt: font_size,
        position: (node_pos.0, top_corner_y),
        bounds: (tl_ink.advance.max(1.0), font_size * 1.5),
        palette_offset: 0,
        cluster_count: count_grapheme_clusters(&border_style.corners.top_left),
    });
    // Channel 6: TR corner.
    specs.push(BorderRunSpec {
        channel: 6,
        text: border_style.corners.top_right.clone(),
        font_size_pt: font_size,
        line_height_pt: font_size,
        position: (node_pos.0 + node_size.0 - tr_ink.advance, top_corner_y),
        bounds: (tr_ink.advance.max(1.0), font_size * 1.5),
        palette_offset: 1 + top_clusters,
        cluster_count: count_grapheme_clusters(&border_style.corners.top_right),
    });
    // Channel 7: BL corner.
    specs.push(BorderRunSpec {
        channel: 7,
        text: border_style.corners.bottom_left.clone(),
        font_size_pt: font_size,
        line_height_pt: font_size,
        position: (node_pos.0, bottom_corner_y),
        bounds: (bl_ink.advance.max(1.0), font_size * 1.5),
        palette_offset: 1 + top_clusters + 1 + right_clusters,
        cluster_count: count_grapheme_clusters(&border_style.corners.bottom_left),
    });
    // Channel 8: BR corner.
    specs.push(BorderRunSpec {
        channel: 8,
        text: border_style.corners.bottom_right.clone(),
        font_size_pt: font_size,
        line_height_pt: font_size,
        position: (node_pos.0 + node_size.0 - br_ink.advance, bottom_corner_y),
        bounds: (br_ink.advance.max(1.0), font_size * 1.5),
        palette_offset: 1 + top_clusters + 1 + right_clusters + 1 + bottom_clusters,
        cluster_count: count_grapheme_clusters(&border_style.corners.bottom_right),
    });
    specs
}

/// First grapheme of a side pattern's fill / cluster. Used by
/// the vertical-rail line-height computation: we measure the
/// first grapheme's ink-height and use it as the per-row
/// y-stride so consecutive rows touch.
fn side_pattern_first_grapheme(pattern: &SidePattern) -> String {
    use crate::mindmap::border_pattern::SidePattern;
    match pattern {
        SidePattern::AtomicRepeat { cluster } => cluster.first().cloned().unwrap_or_default(),
        SidePattern::PrefixFillSuffix { fill, .. } => fill.first().cloned().unwrap_or_default(),
    }
}

/// Bytes a vertical rail spends per **row**, rounded up.
///
/// A rail emits one grapheme cluster per row, but not the *same*
/// one: `SidePattern::render(rows)` writes `rows / unit` copies of
/// the whole cluster vector, so row `i` carries `cluster[i % unit]`.
/// Measuring the first grapheme instead — which is what the byte
/// ceiling used to be handed — reads a pattern like `"A" + 1000
/// combining marks` as one byte per row and lets the ceiling pass a
/// hundred megabytes.
///
/// The mean is exact rather than approximate here, because `render`
/// only ever emits whole copies: `rows / unit` copies of
/// `total_bytes` is `rows × total_bytes / unit`. Rounded up so the
/// ceiling never under-charges a row.
///
/// This prices only the **repeating** part. A `PrefixFillSuffix`
/// pattern also emits a prefix and a suffix once each, which do not
/// scale with the row count and are charged separately — see
/// [`side_pattern_fixed_bytes`]. The horizontal fitter sums its own
/// fill clusters but has the same fixed part, and takes the same
/// treatment.
///
/// Cost: one pass over the pattern's clusters, no allocation.
pub(crate) fn side_pattern_bytes_per_row(pattern: &SidePattern) -> usize {
    use crate::mindmap::border_pattern::SidePattern;
    let clusters: &[String] = match pattern {
        SidePattern::AtomicRepeat { cluster } => cluster.as_slice(),
        SidePattern::PrefixFillSuffix { fill, .. } => fill.as_slice(),
    };
    let unit = clusters.len();
    if unit == 0 {
        return 1;
    }
    let total: usize = clusters.iter().map(|g| g.len()).sum();
    total.div_ceil(unit).max(1)
}

/// Bytes a rail emits **regardless** of its row count.
///
/// `SidePattern::render` writes a `PrefixFillSuffix` pattern's
/// `prefix` and `suffix` once each, around the repeated fill, so
/// neither scales with rows and neither is priced by
/// [`side_pattern_bytes_per_row`]. Charging them separately is what
/// keeps [`MAX_BORDER_SIDE_BYTES`] a bound on the emitted string
/// rather than on its repeating part only.
///
/// An `AtomicRepeat` pattern has no fixed part and reports 0.
///
/// Cost: one pass over the fixed clusters, no allocation.
pub(crate) fn side_pattern_fixed_bytes(pattern: &SidePattern) -> usize {
    use crate::mindmap::border_pattern::SidePattern;
    match pattern {
        SidePattern::AtomicRepeat { .. } => 0,
        SidePattern::PrefixFillSuffix { prefix, suffix, .. } => {
            prefix.iter().map(|g| g.len()).sum::<usize>() + suffix.iter().map(|g| g.len()).sum::<usize>()
        }
    }
}

/// Hard ceiling on the grapheme count one border side may emit.
///
/// The copy count is `available width / cluster advance`, and
/// **both terms are authored**: the width comes from `node.size`
/// and the advance from the border's `font_size_pt`. A
/// `.mindmap.json` is untrusted input, so their quotient is
/// attacker-controlled, and it drives a `String` push loop — a
/// hostile pair asks for a multi-gigabyte string on the scene-build
/// path, which fails as an allocator abort rather than a catchable
/// panic. The loader's numeric-domain check
/// (`model::validate`) already rejects the sizes that make the
/// quotient absurd; this is the second wall.
///
/// A border side spanning a full desktop viewport at a readable
/// size runs to a few hundred graphemes, so this is generous by
/// orders of magnitude and still bounded.
pub const MAX_BORDER_SIDE_GLYPHS: usize = 100_000;

/// Ceiling on the byte length of one border side's emitted string.
///
/// [`MAX_BORDER_SIDE_GLYPHS`] bounds the *grapheme* count, which is
/// the right unit for "how much border is drawn" and the wrong one
/// for "how much memory this costs". A grapheme is not a byte: a ZWJ
/// sequence runs to dozens, and a side pattern is an author-supplied
/// string, so a map can make each emitted cluster arbitrarily large
/// and multiply it by the grapheme cap. Both units are bounded, and
/// whichever binds first wins.
pub const MAX_BORDER_SIDE_BYTES: usize = 1024 * 1024;

/// How many whole copies of a cluster fit in `available_pt`, bounded
/// in **both** units the emitted string is measured in.
///
/// The two ceilings are separate because a grapheme is not a byte:
/// [`MAX_BORDER_SIDE_GLYPHS`] bounds how much border is drawn, and
/// [`MAX_BORDER_SIDE_BYTES`] bounds what it costs, and an
/// author-supplied pattern can make one cluster arbitrarily long in
/// bytes while staying one grapheme. Whichever binds first wins, so
/// `cluster_bytes` is a required argument rather than something a
/// caller can default — a caller that passed the grapheme count here
/// would silently disable the byte ceiling.
///
/// `fixed_bytes` is what the rail emits **regardless** of the copy
/// count — the `prefix` and `suffix` of a `PrefixFillSuffix`
/// pattern, which `render` writes once each around the repeated
/// fill. It is charged against the byte ceiling before the repeats
/// are, because a ceiling that only prices the repeating part does
/// not bound the emitted string: an authored prefix rides straight
/// through it. Pass 0 for a pattern with no fixed part.
///
/// Total over hostile inputs: a non-finite or non-positive advance
/// yields zero copies rather than a saturating cast into the push
/// loop, and a `fixed_bytes` at or over the ceiling yields zero
/// repeats rather than underflowing. Cost: a few float ops, no
/// allocation.
pub(crate) fn fill_copies(
    available_pt: f32,
    cluster_w: f32,
    cluster_len: usize,
    cluster_bytes: usize,
    fixed_bytes: usize,
) -> usize {
    if !available_pt.is_finite() || !cluster_w.is_finite() || cluster_w <= 0.0 || cluster_len == 0 {
        return 0;
    }
    let copies = (available_pt / cluster_w).floor();
    if !copies.is_finite() || copies <= 0.0 {
        return 0;
    }
    let by_graphemes = MAX_BORDER_SIDE_GLYPHS / cluster_len;
    let byte_budget = MAX_BORDER_SIDE_BYTES.saturating_sub(fixed_bytes);
    let by_bytes = byte_budget / cluster_bytes.max(1);
    (copies as usize).min(by_graphemes).min(by_bytes)
}

/// Fit `pattern` into `available_pt` of horizontal space, given
/// the active face's measured per-grapheme advances. Returns
/// `(rendered_text, cluster_count, rendered_width_pt)`. The
/// rendered width is **always ≤ available_pt** — `floor()`
/// rather than `round()` so the fill never overshoots its
/// allocated span and clips into the corner glyph next to it.
///
/// For the demo's `Atomic-repeat` node (size 360×110,
/// font_size_pt 18, top `+=##=+`, corners `┌`/`┐`): the cache
/// returns the actual measured widths of each grapheme in
/// LiberationSans, the sum gives the cluster's true width, and
/// `floor(available / cluster_w)` picks the largest N copies
/// that fit. The leftover sub-cluster pixels stay blank, so the
/// rail terminates flush with the right corner.
fn fit_pattern_to_width(
    font_system: &mut FontSystem,
    pattern: &SidePattern,
    available_pt: f32,
    face: Option<crate::font::fonts::AppFont>,
    font_size: f32,
) -> (String, usize, f32) {
    use crate::font::metric_cache::glyph_advance_with;
    use crate::mindmap::border_pattern::SidePattern;
    match pattern {
        SidePattern::AtomicRepeat { cluster } => {
            if available_pt <= 0.0 || cluster.is_empty() {
                return (String::new(), 0, 0.0);
            }
            // Per-grapheme widths so partial-cluster filling can
            // greedily add graphemes from the cluster until adding
            // the next would overshoot `available_pt`. This gets us
            // sub-grapheme-precision tiling: leftover gap < width
            // of the smallest grapheme in the cluster.
            let g_widths: Vec<f32> = cluster
                .iter()
                .map(|g| glyph_advance_with(font_system, face, font_size, g))
                .collect();
            let cluster_w: f32 = g_widths.iter().sum();
            if cluster_w <= 0.0 {
                return (String::new(), 0, 0.0);
            }
            let cluster_bytes: usize = cluster.iter().map(|g| g.len()).sum();
            // AtomicRepeat has no fixed prefix/suffix.
            let full_copies = fill_copies(available_pt, cluster_w, cluster.len(), cluster_bytes, 0);
            let mut emitted_w = full_copies as f32 * cluster_w;
            let mut text = String::new();
            for _ in 0..full_copies {
                for g in cluster {
                    text.push_str(g);
                }
            }
            let mut cluster_count = full_copies * cluster.len();
            // Greedy partial-cluster fill. Bounded by the byte
            // ceiling as well as by width: `fill_copies` priced the
            // whole copies above, and this loop runs *after* it, so
            // checking width alone let the emitted string overshoot
            // MAX_BORDER_SIDE_BYTES by up to one cluster vector.
            let mut idx = 0;
            while idx < cluster.len() {
                let next_w = g_widths[idx];
                if emitted_w + next_w > available_pt {
                    break;
                }
                if text.len() + cluster[idx].len() > MAX_BORDER_SIDE_BYTES {
                    break;
                }
                text.push_str(&cluster[idx]);
                emitted_w += next_w;
                cluster_count += 1;
                idx += 1;
            }
            (text, cluster_count, emitted_w)
        }
        SidePattern::PrefixFillSuffix { prefix, fill, suffix } => {
            let prefix_widths: Vec<f32> = prefix
                .iter()
                .map(|g| glyph_advance_with(font_system, face, font_size, g))
                .collect();
            let suffix_widths: Vec<f32> = suffix
                .iter()
                .map(|g| glyph_advance_with(font_system, face, font_size, g))
                .collect();
            let fill_widths: Vec<f32> = fill
                .iter()
                .map(|g| glyph_advance_with(font_system, face, font_size, g))
                .collect();
            let prefix_w: f32 = prefix_widths.iter().sum();
            let suffix_w: f32 = suffix_widths.iter().sum();
            let fill_cluster_w: f32 = fill_widths.iter().sum();

            if available_pt < prefix_w + suffix_w {
                // Defensive fallback: degenerate small node.
                let rendered = pattern.render(prefix.len() + suffix.len());
                return (rendered.text, rendered.cluster_count, prefix_w + suffix_w);
            }
            let between_avail = available_pt - prefix_w - suffix_w;
            let fill_bytes: usize = fill.iter().map(|g| g.len()).sum();
            // The prefix and suffix are emitted unconditionally below,
            // so they are charged against the ceiling before the
            // repeats are — otherwise an authored prefix rides
            // straight past a cap that claims to bound the whole
            // emitted string.
            let fixed_bytes: usize =
                prefix.iter().map(|g| g.len()).sum::<usize>() + suffix.iter().map(|g| g.len()).sum::<usize>();
            let full_copies = fill_copies(between_avail, fill_cluster_w, fill.len(), fill_bytes, fixed_bytes);

            let mut text = String::new();
            let mut cluster_count = 0;
            let mut emitted_w = 0.0_f32;

            for g in prefix {
                text.push_str(g);
            }
            cluster_count += prefix.len();
            emitted_w += prefix_w;

            for _ in 0..full_copies {
                for g in fill {
                    text.push_str(g);
                }
            }
            cluster_count += full_copies * fill.len();
            emitted_w += full_copies as f32 * fill_cluster_w;

            // Partial-fill: greedy add fill graphemes until the
            // next would push us past `available_pt - suffix_w`
            // (we have to leave room for the suffix), or past the
            // byte ceiling. The byte half matters for the same
            // reason as in the atomic arm: this loop runs after
            // `fill_copies` has already priced the whole copies, so
            // width alone let the result overshoot. The suffix is
            // still to come, so its bytes are reserved too.
            let mut idx = 0;
            while idx < fill.len() {
                let next_w = fill_widths[idx];
                if emitted_w + next_w + suffix_w > available_pt {
                    break;
                }
                let suffix_bytes: usize = suffix.iter().map(|g| g.len()).sum();
                if text.len() + fill[idx].len() + suffix_bytes > MAX_BORDER_SIDE_BYTES {
                    break;
                }
                text.push_str(&fill[idx]);
                emitted_w += next_w;
                cluster_count += 1;
                idx += 1;
            }

            for g in suffix {
                text.push_str(g);
            }
            cluster_count += suffix.len();
            emitted_w += suffix_w;

            (text, cluster_count, emitted_w)
        }
    }
}

/// Widest single-grapheme measured advance across `pattern`'s
/// cluster. Used to size the buffer width for a vertical rail
/// (`bounds.0`) so cosmic-text doesn't wrap. Slack handling
/// happens in the caller.
fn side_pattern_max_advance(
    font_system: &mut FontSystem,
    pattern: &SidePattern,
    face: Option<crate::font::fonts::AppFont>,
    font_size: f32,
) -> f32 {
    use crate::font::metric_cache::glyph_advance_with;
    use crate::mindmap::border_pattern::SidePattern;
    let graphemes: &[String] = match pattern {
        SidePattern::AtomicRepeat { cluster } => cluster.as_slice(),
        SidePattern::PrefixFillSuffix { fill, .. } => fill.as_slice(),
    };
    graphemes
        .iter()
        .map(|g| glyph_advance_with(font_system, face, font_size, g))
        .fold(0.0_f32, |acc: f32, w: f32| acc.max(w))
}

/// Per-corner cluster counts, used by the auto-resize pass to
/// reason about minimum widths in the same cluster units the
/// pattern fitter speaks in.
#[derive(Clone, Copy, Debug)]
pub struct CornerClusterCounts {
    /// Grapheme-cluster count of the top-left corner string.
    pub top_left: usize,
    /// Grapheme-cluster count of the top-right corner string.
    pub top_right: usize,
    /// Grapheme-cluster count of the bottom-left corner string.
    pub bottom_left: usize,
    /// Grapheme-cluster count of the bottom-right corner string.
    pub bottom_right: usize,
}

impl CornerClusterCounts {
    /// Cluster width consumed by the top corners together.
    pub fn top_horizontal(self) -> usize {
        self.top_left + self.top_right
    }
    /// Cluster width consumed by the bottom corners together.
    pub fn bottom_horizontal(self) -> usize {
        self.bottom_left + self.bottom_right
    }
}

impl Default for BorderStyle {
    fn default() -> Self {
        Self::default_with_color("#ffffff")
    }
}

impl BorderGlyphSet {
    /// Multi-cluster runtime corners derived from this preset's
    /// single-char fields. Each corner is a one-cluster string.
    pub fn corners(&self) -> BorderCorners {
        BorderCorners {
            top_left: self.top_left.to_string(),
            top_right: self.top_right.to_string(),
            bottom_left: self.bottom_left.to_string(),
            bottom_right: self.bottom_right.to_string(),
        }
    }

    /// Side patterns for this preset — each side is the
    /// single-character glyph repeated atomically (one cluster
    /// per "iteration"). Always parses; the inputs are bare ASCII
    /// or single-codepoint scalars by construction.
    pub fn side_patterns(&self) -> SidePatternQuad {
        SidePatternQuad {
            top: parse_legacy_glyph(self.top),
            bottom: parse_legacy_glyph(self.bottom),
            left: parse_legacy_glyph(self.left),
            right: parse_legacy_glyph(self.right),
        }
    }
}

fn parse_legacy_glyph(c: char) -> SidePattern {
    // `SidePattern::parse` would treat '(' / ')' / '\\' specially.
    // The four legacy presets contain none of those, but a future
    // preset could; emit `AtomicRepeat { cluster: vec![c] }`
    // directly so the legacy path stays parse-free.
    SidePattern::AtomicRepeat {
        cluster: vec![c.to_string()],
    }
}

/// Resolve a node's effective `BorderStyle` from its optional
/// `GlyphBorderConfig`, the canvas-level default, and the resolved
/// frame color. Single source of truth — every border-build path
/// (the border pass, the section-frame pass, the clip-AABB pass)
/// goes through this so preset / font / size / color / pattern
/// resolution can't drift between them.
///
/// Cascade for each field, most-specific wins:
/// 1. Per-node `GlyphBorderConfig` (the `cfg` arg).
/// 2. Canvas-level default (the `canvas_default` arg).
/// 3. Hardcoded preset / font / size defaults (light, system,
///    14 pt, the resolved frame color).
///
/// **The `color` field has one more tier than the others**, and it
/// sits between 1 and 2. A node's *own* frame color — its palette
/// group's `frame`, or a per-node override of it, passed as
/// `themed` — is per-node, and a map-wide `canvas.default_border`
/// is not, so the theme wins. Without that placement a single
/// `canvas border color=#888888` flattens every palette-derived
/// frame in the document and no per-node edit short of an explicit
/// `border color` can recover one. `style_frame_color` is the
/// floor beneath all of it, which is why it is a plain `&str`:
/// `NodeStyle` always carries one, so a tier under it could never
/// win. Resolve `themed` with
/// [`MindMap::node_frame_theme_tier`](crate::mindmap::model::MindMap::node_frame_theme_tier)
/// — passing `None` by hand means "this node has no theme", which
/// is a claim rather than a default.
///
/// Pattern parse errors on a configured side fall back to the
/// preset's side glyph for that side and emit a `log::warn!` —
/// per `CODE_CONVENTIONS.md` §9, interactive paths must not panic
/// and a corrupt model value should degrade to a sensible visual.
pub fn resolve_border_style(
    cfg: Option<&GlyphBorderConfig>,
    canvas_default: Option<&GlyphBorderConfig>,
    themed: Option<&str>,
    style_frame_color: &str,
) -> BorderStyle {
    // Field-by-field cascade. `cfg` takes precedence; if a key
    // sits at a meaningful default in `cfg` (e.g. the
    // `default_border_preset` literal "light"), the cascade
    // can't tell the difference between "author chose light"
    // and "author left the field unset". We accept that: the
    // canvas default only contributes when the per-node cfg is
    // absent entirely, mirroring the existing `style.frame_color`
    // cascade.
    let chosen = cfg.or(canvas_default);

    let preset_name = chosen.map(|c| c.preset.as_str()).unwrap_or("light");
    let glyph_set = preset_glyph_set(preset_name);

    // Multi-cluster corners + side patterns: prefer the cfg's
    // `glyphs` payload when the preset is `"custom"`; otherwise
    // fall back to the preset's single-glyph defaults.
    let (corners, side_patterns) = if preset_name.eq_ignore_ascii_case("custom") {
        let custom = chosen
            .and_then(|c| c.glyphs.as_ref())
            .cloned()
            .unwrap_or_else(default_custom_glyphs);
        corners_and_patterns_from_custom(&custom, &glyph_set)
    } else {
        (glyph_set.corners(), glyph_set.side_patterns())
    };

    let font_name = chosen.and_then(|c| c.font.clone());
    let font_size_pt = chosen
        .map(|c| c.font_size_pt)
        .unwrap_or_else(default_border_font_size);
    // The one field whose cascade is not `chosen`-shaped. An
    // explicit per-node `border.color` still wins; below it the
    // node's own theme outranks the map-wide default, and
    // `style_frame_color` is the floor. `chosen` cannot express
    // that, because it has already collapsed the per-node and
    // canvas tiers into one.
    let color = cfg
        .and_then(|c| c.color.clone())
        .or_else(|| themed.map(str::to_string))
        .or_else(|| canvas_default.and_then(|c| c.color.clone()))
        .unwrap_or_else(|| style_frame_color.to_string());
    let color_palette = chosen.and_then(|c| c.color_palette.clone());
    let palette_field =
        PaletteField::from_str_or_default(chosen.and_then(|c| c.color_palette_field.as_deref()));

    BorderStyle {
        glyph_set,
        corners,
        side_patterns,
        color_palette,
        palette_field,
        font_name,
        font_size_pt,
        color,
        visible: true,
    }
}

/// The `font_size_pt` half of [`resolve_border_style`]'s cascade,
/// on its own.
///
/// A caller that only needs the border's *extent* — the clip-AABB
/// pass behind
/// [`crate::mindmap::tree_builder::node_clip_aabbs`], which grows a
/// node's clip box by the rendered frame's thickness — would
/// otherwise pay for a whole [`BorderStyle`]: four corner
/// `String`s, four `SidePattern`s each carrying a `Vec`, the color
/// `String`, and the optional font / palette clones, all discarded
/// after reading one `f32`.
///
/// **Parity contract:** this must stay byte-identical to what
/// [`resolve_border_style`] writes into
/// [`BorderStyle::font_size_pt`] — same `cfg.or(canvas_default)`
/// chosen-slot rule, same `default_border_font_size()` floor. It deliberately takes no
/// `frame_color`: that argument feeds only `BorderStyle::color`,
/// and the size cascade is independent of both it and the preset.
/// `resolve_border_font_size_pt_matches_resolve_border_style` pins
/// the equivalence across presets, per-node overrides,
/// canvas-default fall-through, and the unset floor.
///
/// # Costs
///
/// Two `Option` derefs and a copy. No allocation, no parsing.
pub fn resolve_border_font_size_pt(
    cfg: Option<&GlyphBorderConfig>,
    canvas_default: Option<&GlyphBorderConfig>,
) -> f32 {
    cfg.or(canvas_default)
        .map(|c| c.font_size_pt)
        .unwrap_or_else(default_border_font_size)
}

/// Resolve a section-frame's [`BorderStyle`] against the same
/// vocabulary node borders use. Drives the cyan rectangle drawn
/// around each section while the owning node is in
/// `InteractionMode::NodeEdit` mode (Plan §3.5 / §4.3).
///
/// Cascade (mirrors [`resolve_border_style`] but with section-
/// frame-specific canvas defaults and floors):
/// 1. `section.frame_border` if `Some` — per-section author override.
/// 2. else `canvas.default_section_frame_border` (or
///    `canvas.default_focused_section_frame_border` when `focused`)
///    if `Some` — map-wide author default.
/// 3. else a hardcoded floor `GlyphBorderConfig`: `light` preset
///    for unfocused sections, `heavy` for the focused section.
///    Synthesized so the cascade always routes through one
///    `resolve_border_style` call — there are no inline glyph
///    constants in the section-frame path.
///
/// `frame_color_resolved` is the cyan `SELECTED_EDGE_COLOR` the
/// caller already resolved through `theme_variables`. The frame
/// system is mode-driven chrome — the active-affordance signal
/// (cyan) sits at the bottom of the cascade; an author who sets
/// `section.frame_border.color = "#ff8800"` overrides the cyan
/// fully, which is the desired shape for "make my borders tell a
/// story". Authors who want the active-affordance signal preserved
/// just leave `color` unset on their override.
pub fn resolve_section_frame_border(
    section: &MindSection,
    canvas: &Canvas,
    focused: bool,
    frame_color_resolved: &str,
) -> BorderStyle {
    // Field-by-field cascade — same shape as `resolve_border_style`.
    // Per-section override wins; otherwise the canvas-level default
    // for the focused / unfocused state. When neither is set we
    // synthesize a hardcoded floor `GlyphBorderConfig` so the
    // returned `BorderStyle` flows through the same resolver every
    // other border consumes — the floor is just another author
    // default that authors can override at any cascade level.
    let canvas_default = if focused {
        canvas
            .default_focused_section_frame_border
            .as_ref()
            .or(canvas.default_section_frame_border.as_ref())
    } else {
        canvas.default_section_frame_border.as_ref()
    };
    let chosen: &GlyphBorderConfig = section
        .frame_border
        .as_ref()
        .or(canvas_default)
        .unwrap_or_else(|| section_frame_floor_config(focused));
    // `None` for the theme tier: `chosen` has already collapsed
    // the per-section and canvas-section-frame tiers, and there is
    // no canvas default left for a theme to outrank.
    resolve_border_style(Some(chosen), None, None, frame_color_resolved)
}

/// Hardcoded fallback `GlyphBorderConfig` for section frames when
/// both the per-section override and the canvas default are unset.
/// Synthesized so the resolver path is the same for every section
/// frame regardless of where its style comes from.
///
/// `focused = false` → light preset (┌─┐│└─┘).
/// `focused = true`  → heavy preset (┏━┓┃┗━┛).
pub fn section_frame_floor_config(focused: bool) -> &'static GlyphBorderConfig {
    use std::sync::OnceLock;
    static UNFOCUSED: OnceLock<GlyphBorderConfig> = OnceLock::new();
    static FOCUSED: OnceLock<GlyphBorderConfig> = OnceLock::new();
    let init = |preset: &'static str| GlyphBorderConfig {
        preset: preset.to_string(),
        font: None,
        font_size_pt: SECTION_FRAME_FLOOR_FONT_SIZE_PT,
        color: None,
        glyphs: None,
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    };
    if focused {
        FOCUSED.get_or_init(|| init("heavy"))
    } else {
        UNFOCUSED.get_or_init(|| init("light"))
    }
}

/// Font size (pt) baked into the hardcoded floor
/// `GlyphBorderConfig` returned by `section_frame_floor_config`.
/// Smaller than the `GlyphBorderConfig` field default (14 pt) so
/// the per-section subdivisions read as a finer-grained
/// subdivision rather than competing with the node frame in the
/// no-author-config path.
///
/// **Cascade caveat.** Authors who set a partial canvas-level
/// override like `default_section_frame_border = { preset:
/// "double" }` get the **`GlyphBorderConfig` field default of
/// 14 pt**, not this 10 pt — `font_size_pt` is `f32`, not
/// `Option<f32>`, so deserialization can't distinguish "author
/// omitted" from "author wrote 14.0". Authors who want the
/// smaller-grain feel inside their config write
/// `font_size_pt: 10.0` (or any value they prefer) explicitly.
/// This matches the pattern node borders use today and keeps
/// the cascade lossless.
const SECTION_FRAME_FLOOR_FONT_SIZE_PT: f32 = 10.0;

/// Pick the [`BorderGlyphSet`] for a preset name, case-insensitively.
/// Unknown preset names fall back to `light` and log a warning;
/// the `"custom"` preset returns `light` here too — its corners /
/// sides are overridden downstream by the `glyphs` payload, so the
/// fallback only shows through when the user picked custom but
/// supplied no glyph fields.
///
/// Reachable per tree rebuild (`tree_builder/border.rs:111`),
/// which §9 puts in interactive territory — no panic site. The
/// fallback is statically known to exist because `PRESET_TABLE`
/// is `const &[…; 4]` (non-empty by construction).
pub fn preset_glyph_set(preset: &str) -> BorderGlyphSet {
    let name = preset.to_ascii_lowercase();
    let row = PRESET_TABLE
        .iter()
        .find(|(n, _, _)| *n == name)
        .unwrap_or_else(|| {
            // "custom" is in `BORDER_PRESETS` but absent from the
            // glyph table — it signals "user-supplied glyphs
            // override these defaults," with the per-side fallback
            // to `light`. Anything else gets a warn-log.
            if name != CUSTOM_PRESET_NAME {
                log::warn!("mindmap::border: preset '{}' unknown; using 'light'", preset);
            }
            &PRESET_TABLE[0]
        });
    BorderGlyphSet::from_glyphs(row.1)
}

/// Sentinel preset name for "user-supplied glyphs override these
/// defaults." Has no row in `PRESET_TABLE`; the per-side
/// fallback in [`preset_glyph_set`] is `light`. Repeated literal
/// `"custom"` checks reach for this to keep the meaning single-
/// sourced.
pub const CUSTOM_PRESET_NAME: &str = "custom";

/// Description of [`CUSTOM_PRESET_NAME`] for
/// [`border_preset_hint`]. Lives beside the sentinel rather than in
/// `PRESET_TABLE` for the same reason the sentinel does: `custom`
/// has no glyph row.
const CUSTOM_PRESET_HINT: &str = "user-supplied per-side / per-corner glyphs";

/// One-line human description of a preset name, for a UI that
/// offers presets to pick from — the console's `border preset=`
/// completion is the consumer today.
///
/// Matching is case-insensitive, mirroring [`preset_glyph_set`].
/// Returns `None` for a name that is not in [`BORDER_PRESETS`];
/// every name that *is* in `BORDER_PRESETS` returns `Some`, because
/// both this lookup and that list derive from the same
/// `PRESET_TABLE` (plus the `custom` sentinel). That is what keeps a
/// newly added preset from silently completing with a blank
/// description.
///
/// O(n) over the four-row table, no allocation beyond the
/// lowercased needle.
pub fn border_preset_hint(preset: &str) -> Option<&'static str> {
    let name = preset.to_ascii_lowercase();
    BORDER_PRESET_ROWS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, hint)| *hint)
}

/// Every preset name paired with its one-line description, in the
/// order [`BORDER_PRESETS`] lists them.
///
/// The pair form is what a UI needs when it offers the presets as a
/// *vocabulary* rather than looking one up: the console's
/// declarative grammar (`console::spec`) builds its `preset` word
/// list from this at const-fn time, so a fifth row in
/// `PRESET_TABLE` reaches the popup, the usage line and the
/// rejection message with its description already attached.
/// [`BORDER_PRESETS`] and [`border_preset_hint`] both read it, so
/// the three views cannot come apart.
pub const BORDER_PRESET_ROWS: &[(&str, &str)] = &{
    const N: usize = PRESET_TABLE.len();
    let mut out: [(&str, &str); N + 1] = [("", ""); N + 1];
    let mut i = 0;
    while i < N {
        out[i] = (PRESET_TABLE[i].0, PRESET_TABLE[i].2);
        i += 1;
    }
    out[N] = (CUSTOM_PRESET_NAME, CUSTOM_PRESET_HINT);
    out
};

/// Every preset name accepted by the schema's
/// `GlyphBorderConfig.preset` field — the four typed glyph rows
/// in `PRESET_TABLE` plus the [`CUSTOM_PRESET_NAME`] sentinel.
/// Surfaced for the console's `border preset=` completion.
///
/// Derived from [`BORDER_PRESET_ROWS`]'s name column at const-fn
/// time so adding a fifth glyph row to `PRESET_TABLE` only
/// requires extending the table — `BORDER_PRESETS` rebuilds
/// automatically.
pub const BORDER_PRESETS: &[&str] = &{
    const N: usize = BORDER_PRESET_ROWS.len();
    let mut out: [&str; N] = [""; N];
    let mut i = 0;
    while i < N {
        out[i] = BORDER_PRESET_ROWS[i].0;
        i += 1;
    }
    out
};

/// Advance to the next preset in [`BORDER_PRESETS`], wrapping
/// at the end. Used by the `border preset cycle` console verb
/// and the `Action::CycleBorderPreset` dispatch arm — single
/// source of truth for the wrap order so the verb and the
/// keybind can't drift. Unknown `current` falls through to the
/// last entry, so the next call lands at index 0 (`light`).
pub fn next_border_preset(current: &str) -> &'static str {
    let idx = BORDER_PRESETS
        .iter()
        .position(|p| p.eq_ignore_ascii_case(current))
        .unwrap_or(BORDER_PRESETS.len() - 1);
    BORDER_PRESETS[(idx + 1) % BORDER_PRESETS.len()]
}

fn corners_and_patterns_from_custom(
    g: &CustomBorderGlyphs,
    fallback: &BorderGlyphSet,
) -> (BorderCorners, SidePatternQuad) {
    let corners = BorderCorners {
        top_left: nonempty_or(&g.top_left, fallback.top_left),
        top_right: nonempty_or(&g.top_right, fallback.top_right),
        bottom_left: nonempty_or(&g.bottom_left, fallback.bottom_left),
        bottom_right: nonempty_or(&g.bottom_right, fallback.bottom_right),
    };
    let side_patterns = SidePatternQuad {
        top: parse_side_or_fallback(&g.top, fallback.top, "top"),
        bottom: parse_side_or_fallback(&g.bottom, fallback.bottom, "bottom"),
        left: parse_side_or_fallback(&g.left, fallback.left, "left"),
        right: parse_side_or_fallback(&g.right, fallback.right, "right"),
    };
    (corners, side_patterns)
}

fn nonempty_or(s: &str, fallback: char) -> String {
    if s.is_empty() {
        fallback.to_string()
    } else {
        s.to_string()
    }
}

fn parse_side_or_fallback(s: &str, fallback: char, side: &str) -> SidePattern {
    if s.is_empty() {
        return parse_legacy_glyph(fallback);
    }
    match SidePattern::parse(s) {
        Ok(p) => p,
        Err(e) => {
            log::warn!(
                "border {} pattern '{}' rejected ({}); falling back to preset",
                side,
                s,
                e
            );
            parse_legacy_glyph(fallback)
        }
    }
}

pub fn default_custom_glyphs() -> CustomBorderGlyphs {
    // Mirrors the `default_*_glyph` defaults on the data-model
    // type. Used when `preset = "custom"` but `glyphs` is absent.
    // Corners are the light preset's `┌┐└┘` rather than the
    // rounded `╭╮╰╯` so the fallback joins cleanly with the
    // side glyphs at cell boundaries.
    CustomBorderGlyphs {
        top: "\u{2500}".to_string(),
        bottom: "\u{2500}".to_string(),
        left: "\u{2502}".to_string(),
        right: "\u{2502}".to_string(),
        top_left: "\u{250C}".to_string(),
        top_right: "\u{2510}".to_string(),
        bottom_left: "\u{2514}".to_string(),
        bottom_right: "\u{2518}".to_string(),
    }
}

/// Apply a [`crate::mindmap::tree_builder::BorderConfigEditsView`]
/// to a slot for live-preview rendering. Mirrors the application-
/// crate's `apply_glyph_border_edits_to_slot` shape but consumes
/// borrowed strings rather than `OptionEdit<T>` so the border and
/// section-frame passes can fold the staged preview edits into a
/// clone of the committed slot without round-tripping back through
/// the application layer.
///
/// **Parity contract:** this function must produce the same
/// post-state as `apply_glyph_border_edits_to_slot` for any
/// committing edit. Both paths derive from the same field rules:
/// per-field set-or-keep, side / corner edits force preset to
/// `"custom"`, the `glyphs` slot materializes on first edit. A
/// parity regression here means the preview lies about what
/// commit will produce — Risk #1 in the plan.
///
/// `view.clear == true` empties the slot and short-circuits.
/// Otherwise the helper materializes a fresh
/// `GlyphBorderConfig::default()` on first edit (the same base
/// the committing path materializes) and folds each per-field
/// override.
pub fn apply_view_to_slot(
    slot: &mut Option<GlyphBorderConfig>,
    view: &crate::mindmap::tree_builder::BorderConfigEditsView<'_>,
) {
    use crate::mindmap::tree_builder::EditView;
    // Top-level slot clear — empties the entire slot, falls back
    // to the canvas default / hardcoded floor on resolve.
    if view.clear {
        *slot = None;
        return;
    }
    if !view.touches_any_field() {
        return;
    }
    let cfg = slot.get_or_insert_with(GlyphBorderConfig::default);
    // Per-field tri-state apply. `Keep` is no-op; `Clear` drops
    // the field's `Option<String>` (or leaves a non-Option field
    // at its default for `font_size_pt` / `padding`); `Set` writes
    // the value. Mirrors the application-side
    // `apply_glyph_border_edits_to_slot` field-by-field.
    if let EditView::Set(p) = view.preset {
        cfg.preset = p.to_string();
    }
    match view.font {
        EditView::Keep => {}
        EditView::Clear => cfg.font = None,
        EditView::Set(f) => cfg.font = Some(f.to_string()),
    }
    if let EditView::Set(s) = view.font_size_pt {
        cfg.font_size_pt = s;
    }
    match view.color {
        EditView::Keep => {}
        EditView::Clear => cfg.color = None,
        EditView::Set(c) => cfg.color = Some(c.to_string()),
    }
    if let EditView::Set(p) = view.padding {
        cfg.padding = p;
    }
    match view.color_palette {
        EditView::Keep => {}
        EditView::Clear => cfg.color_palette = None,
        EditView::Set(p) => cfg.color_palette = Some(p.to_string()),
    }
    match view.color_palette_field {
        EditView::Keep => {}
        EditView::Clear => cfg.color_palette_field = None,
        EditView::Set(f) => cfg.color_palette_field = Some(f.to_string()),
    }
    if view.touches_glyphs() {
        if cfg.glyphs.is_none() {
            cfg.glyphs = Some(default_custom_glyphs());
        }
        if !cfg.preset.eq_ignore_ascii_case("custom") {
            cfg.preset = "custom".to_string();
        }
        let g = cfg.glyphs.as_mut().expect("just inserted");
        // Side / corner glyphs are non-`Option<String>` on the
        // model side (`CustomBorderGlyphs`), so `Clear` semantics
        // for them mean "fall back to the preset's default char"
        // — same posture the application-side helper takes
        // (`apply_string_set` with no Clear handling).
        if let EditView::Set(v) = view.side_top {
            g.top = v.to_string();
        }
        if let EditView::Set(v) = view.side_bottom {
            g.bottom = v.to_string();
        }
        if let EditView::Set(v) = view.side_left {
            g.left = v.to_string();
        }
        if let EditView::Set(v) = view.side_right {
            g.right = v.to_string();
        }
        if let EditView::Set(v) = view.corner_top_left {
            g.top_left = v.to_string();
        }
        if let EditView::Set(v) = view.corner_top_right {
            g.top_right = v.to_string();
        }
        if let EditView::Set(v) = view.corner_bottom_left {
            g.bottom_left = v.to_string();
        }
        if let EditView::Set(v) = view.corner_bottom_right {
            g.bottom_right = v.to_string();
        }
    }
}

/// Resolve `border_style.color_palette` (a name) to a list of
/// per-cycle-position RGBA colors, reading the configured
/// `palette_field` channel out of each `ColorGroup`. Returns an
/// empty `Vec` when the name is unset or the named palette is not
/// in the map (logs a warning in the latter case per
/// `CODE_CONVENTIONS.md` §9). Pre-resolution lets the renderer and
/// tree builder consume the color list without re-walking the
/// palette map and re-parsing its hex strings — [`build_border_regions`]
/// indexes the returned slice once per glyph cluster, so the walk
/// happens once per border rather than once per glyph.
///
/// Cost: O(groups.len()) hex parses on names that resolve, O(1) on
/// the unset / missing fallback paths.
pub fn resolve_palette_cycle(
    palettes: &std::collections::HashMap<String, crate::mindmap::model::Palette>,
    border_style: &BorderStyle,
    fallback_rgba: FloatRgba,
) -> Vec<FloatRgba> {
    let Some(name) = border_style.color_palette.as_deref() else {
        return Vec::new();
    };
    let Some(palette) = palettes.get(name) else {
        log::warn!(
            "border color_palette '{}' not found in map; falling back to single color",
            name
        );
        return Vec::new();
    };
    palette
        .groups
        .iter()
        .map(|g| {
            let hex = border_style.palette_field.read(g);
            crate::util::color::hex_to_rgba_safe(hex, fallback_rgba)
        })
        .collect()
}

/// Build a [`ColorFontRegions`] that paints `cluster_count` glyph
/// clusters. When `palette_cycle` is non-empty, each cluster
/// picks its color from `palette_cycle[(offset + i) % len]`. When
/// it's empty, a single uniform region is emitted using
/// `fallback_rgba`.
///
/// `glyph_index_offset` lets callers chain side runs into one
/// continuous cycle around the rectangle (top → right → bottom →
/// left), so a color sweep wraps cleanly across corners.
///
/// # Newlines in vertical sides
///
/// Vertical-side text is one cluster per line (`"|\n|\n|"` for a
/// 3-row column) — newline `'\n'` is its own grapheme cluster, so
/// `cluster_count` includes them and the palette index advances
/// across newlines too. The visible glyphs end up at palette
/// positions `[offset, offset+2, offset+4, …]` rather than
/// `[offset, offset+1, offset+2, …]`. This matches the tree
/// builder's per-side region emission, which means the flat-scene
/// renderer and the Baumhard-tree renderer paint identical color
/// sequences. Callers that want a denser cycle on a column can
/// shorten the palette to compensate.
///
/// Cost: O(cluster_count) when palette-cycling, O(1) otherwise.
/// Per-cluster regions are only built when the caller opts in
/// (non-empty cycle) — the default single-region path matches
/// the existing renderer cost.
pub fn build_border_regions(
    cluster_count: usize,
    palette_cycle: &[FloatRgba],
    fallback_rgba: FloatRgba,
    glyph_index_offset: usize,
) -> ColorFontRegions {
    let mut regions = ColorFontRegions::new_empty();
    if cluster_count == 0 {
        return regions;
    }
    if palette_cycle.is_empty() {
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, cluster_count),
            None,
            Some(fallback_rgba),
        ));
        return regions;
    }
    for i in 0..cluster_count {
        let pos = (glyph_index_offset + i) % palette_cycle.len();
        regions.submit_region(ColorFontRegion::new(
            Range::new(i, i + 1),
            None,
            Some(palette_cycle[pos]),
        ));
    }
    regions
}

/// Render a side pattern as a vertical column of `rows` rows.
/// Each cluster sits on its own line; lines are separated by
/// `'\n'` with no trailing newline, matching the existing
/// `BorderGlyphSet::side_border` shape consumed by the renderer.
fn build_vertical_text(pattern: &SidePattern, rows: usize) -> String {
    if rows == 0 {
        return String::new();
    }
    let rendered = pattern.render(rows);
    join_graphemes(&rendered.text, "\n")
}

/// The newline a vertical rail spends per row.
///
/// [`build_vertical_text`] joins the rendered clusters with `'\n'`,
/// which adds `cluster_count - 1` bytes *after* `fill_copies` has
/// applied the byte ceiling. Charging it as part of the per-row
/// cost is what keeps [`MAX_BORDER_SIDE_BYTES`] a bound on the
/// string the rail actually emits rather than on the string before
/// it is joined — at the 100,000-row grapheme ceiling the
/// separators alone are ~100 KB.
const VERTICAL_ROW_SEPARATOR_BYTES: usize = 1;
