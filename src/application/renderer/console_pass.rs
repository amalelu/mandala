//! Console overlay buffer + mutator builders. Stable-channel scheme
//! that mirrors the picker's discipline: every console GlyphArea
//! sits at a deterministic channel so the §B2 in-place mutator path
//! can target it across keystrokes. `console_overlay_areas` is the
//! single source of truth — both the initial-build path
//! (`build_console_overlay_tree`) and the in-place update path
//! (`build_console_overlay_mutator`) consume its output so the two
//! paths cannot drift.


use cosmic_text::FontSystem;
use glam::Vec2;

use baumhard::core::primitives::{ColorFontRegions, Range as ColorFontRange, ColorFontRegion};
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;

use super::console_geometry::{
    build_console_border_strings, lerp_alpha, side_row_count, with_alpha, ConsoleFrameLayout,
    ConsoleOverlayGeometry, ConsoleOverlayLineKind,
};
use super::measure_max_glyph_advance;


// =============================================================
// Stable channel scheme for the console overlay tree
// =============================================================
//
// Mirrors the picker's stable-channel discipline (commit
// `ceaeeb4`): every console GlyphArea sits at a deterministic
// channel so the §B2 in-place mutator path can target it across
// keystrokes. Bands are wide enough to add new sub-rows without
// renumbering. **Order matters** — the values must be strictly
// ascending in tree-insertion order, otherwise Baumhard's
// `align_child_walks` breaks alignment and the mutator path
// silently misses elements.
//
// Layout-wise: 4 borders → `scrollback_rows` × (gutter + text)
// always-emitted slots → `completion_rows` always-emitted slots
// → prompt line. Slots beyond what the geometry currently
// populates carry empty `""` text, which the walker shapes as
// nothing — a stable element set even when scrollback is short.

const CONSOLE_CHANNEL_TOP_BORDER: usize = 1;
const CONSOLE_CHANNEL_BOTTOM_BORDER: usize = 2;
const CONSOLE_CHANNEL_LEFT_COL: usize = 3;
const CONSOLE_CHANNEL_RIGHT_COL: usize = 4;
const CONSOLE_CHANNEL_SCROLLBACK_GUTTER_BASE: usize = 100;
const CONSOLE_CHANNEL_SCROLLBACK_TEXT_BASE: usize = 1_000;
const CONSOLE_CHANNEL_COMPLETION_BASE: usize = 10_000;
const CONSOLE_CHANNEL_PROMPT: usize = 100_000;

/// Single source of truth for the console overlay's GlyphArea
/// content, keyed by stable channel. Both
/// [`build_console_overlay_tree`] (the initial-build path) and
/// [`build_console_overlay_mutator`] (the in-place §B2 update path)
/// consume this so the two paths cannot drift.
///
/// Emits `4 + scrollback_rows * 2 + completion_rows + 1` leaves:
/// the 4 border areas, a `(gutter, text)` pair per scrollback row,
/// one area per completion row, and one prompt. `scrollback_rows`
/// and `completion_rows` are the layout's capped counts — they
/// scale with the underlying geometry, so a count change
/// (scrollback-grow, a Tab firing a new completion list, window
/// resize that enlarges the frame) shifts the structural
/// signature and forces a full rebuild. The mutator path runs
/// whenever the signature is unchanged, which covers keystroke
/// input, completion-highlight cycling, and scrollback-alpha
/// changes as the visible window slides.
///
/// **Channel ordering invariant**: returned in strictly ascending
/// channel order — the constants above are deliberately spaced so
/// that strict order is preserved even when the per-row counts
/// grow.
pub(super) fn console_overlay_areas(
    geometry: &ConsoleOverlayGeometry,
    layout: &ConsoleFrameLayout,
    font_system: &mut FontSystem,
) -> Vec<(usize, GlyphArea)> {
    use crate::application::console::visuals::{
        ACCENT_COLOR, BORDER_COLOR, CURSOR_GLYPH, ERROR_COLOR, GUTTER_GLYPH, INPUT_ECHO_COLOR,
        SCROLLBACK_MIN_ALPHA, SELECTED_COMPLETION_MARKER, TEXT_COLOR,
        UNSELECTED_COMPLETION_MARKER,
    };

    let &ConsoleFrameLayout {
        left,
        top,
        frame_width,
        frame_height,
        font_size,
        char_width,
        row_height,
        inner_padding,
        scrollback_rows,
        completion_rows,
    } = layout;

    let mk_area = |text: &str,
                   color: cosmic_text::Color,
                   font_size: f32,
                   line_height: f32,
                   pos: (f32, f32),
                   bounds: (f32, f32)|
     -> GlyphArea {
        let mut area = GlyphArea::new_with_str(
            text,
            font_size,
            line_height,
            Vec2::new(pos.0, pos.1),
            Vec2::new(bounds.0, bounds.1),
        );
        let rgba = [
            color.r() as f32 / 255.0,
            color.g() as f32 / 255.0,
            color.b() as f32 / 255.0,
            color.a() as f32 / 255.0,
        ];
        area.regions = ColorFontRegions::single_span(
            baumhard::util::grapheme_chad::count_grapheme_clusters(text),
            Some(rgba),
            None,
        );
        area
    };

    let measured_char_width =
        measure_max_glyph_advance(font_system, &["\u{2500}", "\u{2502}"], font_size);
    let cols = ((frame_width / measured_char_width).floor() as usize).max(2);
    let side_rows = side_row_count(frame_height, font_size, row_height);
    let (top_border, bottom_border, left_col, right_col) =
        build_console_border_strings(cols, side_rows);

    let mut out: Vec<(usize, GlyphArea)> = Vec::new();

    // Borders (always present).
    out.push((
        CONSOLE_CHANNEL_TOP_BORDER,
        mk_area(
            &top_border,
            BORDER_COLOR,
            font_size,
            font_size,
            (left, top),
            (frame_width, font_size * 1.5),
        ),
    ));
    out.push((
        CONSOLE_CHANNEL_BOTTOM_BORDER,
        mk_area(
            &bottom_border,
            BORDER_COLOR,
            font_size,
            font_size,
            (left, top + frame_height),
            (frame_width, font_size * 1.5),
        ),
    ));
    out.push((
        CONSOLE_CHANNEL_LEFT_COL,
        mk_area(
            &left_col,
            BORDER_COLOR,
            font_size,
            row_height,
            (left, top + font_size),
            (measured_char_width, frame_height),
        ),
    ));
    let right_col_x = left + (cols.saturating_sub(1) as f32) * measured_char_width;
    out.push((
        CONSOLE_CHANNEL_RIGHT_COL,
        mk_area(
            &right_col,
            BORDER_COLOR,
            font_size,
            row_height,
            (right_col_x, top + font_size),
            (measured_char_width, frame_height),
        ),
    ));

    // Scrollback rows: always emit `scrollback_rows` slots,
    // padding with empty text when the geometry has fewer items.
    // Stable structure is what lets the §B2 mutator path target
    // the same channel across calls when the visible count
    // shifts under it.
    let gutter_x = left + measured_char_width;
    let content_left = gutter_x + measured_char_width + inner_padding;
    let content_width = right_col_x - content_left - inner_padding;
    let content_top = top + font_size + inner_padding;
    let content_cols = (content_width / measured_char_width).floor() as usize;

    // `scrollback_rows` is `min(scrollback.len(), MAX)`, so every
    // `slot in 0..scrollback_rows` maps to a populated entry via
    // `skip + slot`. Signature dispatch (`(scrollback_rows,
    // completion_rows)`) folds count changes into a structural
    // shift that takes the full-rebuild path; the in-place mutator
    // path therefore only runs when the count is unchanged, and
    // every slot inside the loop is `Some`.
    let skip = geometry.scrollback.len().saturating_sub(scrollback_rows);
    let visible_count = scrollback_rows.max(1);
    for slot in 0..scrollback_rows {
        let line = geometry
            .scrollback
            .get(skip + slot)
            .expect("slot index derived from scrollback_rows is always in-bounds");
        let y = content_top + row_height * slot as f32;
        let (gutter_text, gutter_color, text_str, text_color) = {
            let newness = if visible_count <= 1 {
                1.0
            } else {
                slot as f32 / (visible_count - 1) as f32
            };
            let alpha = lerp_alpha(SCROLLBACK_MIN_ALPHA, 0xff, newness);
            let (text_color, gutter_color, gutter_glyph) = match line.kind {
                ConsoleOverlayLineKind::Input => (
                    with_alpha(INPUT_ECHO_COLOR, alpha),
                    with_alpha(INPUT_ECHO_COLOR, alpha),
                    " ",
                ),
                ConsoleOverlayLineKind::Output => (
                    with_alpha(TEXT_COLOR, alpha),
                    with_alpha(ACCENT_COLOR, alpha),
                    GUTTER_GLYPH,
                ),
                ConsoleOverlayLineKind::Error => (
                    with_alpha(ERROR_COLOR, alpha),
                    with_alpha(ERROR_COLOR, alpha),
                    GUTTER_GLYPH,
                ),
            };
            let clipped = baumhard::util::grapheme_chad::truncate_to_display_width(
                &line.text,
                content_cols,
            );
            let gutter = if gutter_glyph == " " {
                String::new()
            } else {
                gutter_glyph.to_string()
            };
            (gutter, gutter_color, clipped.to_string(), text_color)
        };
        out.push((
            CONSOLE_CHANNEL_SCROLLBACK_GUTTER_BASE + slot,
            mk_area(
                &gutter_text,
                gutter_color,
                font_size,
                row_height,
                (gutter_x, y),
                (char_width, row_height),
            ),
        ));
        out.push((
            CONSOLE_CHANNEL_SCROLLBACK_TEXT_BASE + slot,
            mk_area(
                &text_str,
                text_color,
                font_size,
                row_height,
                (content_left, y),
                (content_width, row_height),
            ),
        ));
    }

    // Completion popup rows: same guarantee as scrollback —
    // `completion_rows = min(completions.len(), MAX)`, so every
    // slot index is `Some`.
    let completion_top = content_top + row_height * scrollback_rows as f32;
    for slot in 0..completion_rows {
        let c = geometry
            .completions
            .get(slot)
            .expect("slot index derived from completion_rows is always in-bounds");
        let y = completion_top + row_height * slot as f32;
        let (text_str, color) = {
            let is_selected = geometry.selected_completion == Some(slot);
            let color = if is_selected { ACCENT_COLOR } else { TEXT_COLOR };
            let prefix = if is_selected {
                SELECTED_COMPLETION_MARKER
            } else {
                UNSELECTED_COMPLETION_MARKER
            };
            let line = match &c.hint {
                Some(hint) => format!("{prefix}{}    {}", c.text, hint),
                None => format!("{prefix}{}", c.text),
            };
            let clipped = baumhard::util::grapheme_chad::truncate_to_display_width(
                &line,
                content_cols,
            );
            (clipped.to_string(), color)
        };
        out.push((
            CONSOLE_CHANNEL_COMPLETION_BASE + slot,
            mk_area(
                &text_str,
                color,
                font_size,
                row_height,
                (content_left, y),
                (content_width, row_height),
            ),
        ));
    }

    // Prompt line — single GlyphArea with two ColorFontRegions so
    // the prompt and the input share one shaped run, and the
    // input's first glyph lands at the prompt's actual shaped
    // advance.
    let prompt_budget = font_size * 1.4;
    let y = layout.prompt_y();
    let cursor_byte = baumhard::util::grapheme_chad::find_byte_index_of_grapheme(
        &geometry.input,
        geometry.cursor_grapheme,
    )
    .unwrap_or(geometry.input.len());
    let (pre, post) = geometry.input.split_at(cursor_byte);
    let input_with_cursor = format!("{pre}{CURSOR_GLYPH}{post}");
    let input_clipped = baumhard::util::grapheme_chad::truncate_to_display_width(
        &input_with_cursor,
        content_cols.saturating_sub(2),
    );
    let prompt_text = "\u{276F} ";
    let combined = format!("{prompt_text}{input_clipped}");
    let prompt_chars = baumhard::util::grapheme_chad::count_grapheme_clusters(prompt_text);
    let input_chars =
        baumhard::util::grapheme_chad::count_grapheme_clusters(&input_clipped);

    let mut prompt_area = GlyphArea::new_with_str(
        &combined,
        font_size,
        font_size,
        Vec2::new(content_left, y),
        Vec2::new(content_width, prompt_budget),
    );
    let mut regions = ColorFontRegions::new_empty();
    let to_rgba = |c: cosmic_text::Color| -> [f32; 4] {
        [
            c.r() as f32 / 255.0,
            c.g() as f32 / 255.0,
            c.b() as f32 / 255.0,
            c.a() as f32 / 255.0,
        ]
    };
    regions.submit_region(ColorFontRegion::new(
        ColorFontRange::new(0, prompt_chars),
        None,
        Some(to_rgba(ACCENT_COLOR)),
    ));
    if input_chars > 0 {
        regions.submit_region(ColorFontRegion::new(
            ColorFontRange::new(prompt_chars, prompt_chars + input_chars),
            None,
            Some(to_rgba(TEXT_COLOR)),
        ));
    }
    prompt_area.regions = regions;
    out.push((CONSOLE_CHANNEL_PROMPT, prompt_area));

    out
}

/// Build the console overlay tree from a geometry + pre-computed
/// layout. One Void root with one GlyphArea per stable-channel
/// slot: 4 borders, `scrollback_rows × 2` scrollback slots,
/// `completion_rows` completion slots, and 1 prompt line.
/// Empty slots carry empty text so the structure is constant
/// across keystrokes — the prerequisite for the in-place
/// [`build_console_overlay_mutator`] path.
///
/// Used by [`Renderer::rebuild_console_overlay_buffers`] which
/// then registers the tree under
/// [`crate::application::scene_host::OverlayRole::Console`] and
/// walks it through the standard overlay-scene pipeline.
///
/// `font_system` is needed only for `measure_max_glyph_advance` —
/// no shaping happens here. The returned tree's GlyphArea
/// positions are absolute screen coordinates so the walker's
/// per-tree offset can be `Vec2::ZERO`.
pub(super) fn build_console_overlay_tree(
    geometry: &ConsoleOverlayGeometry,
    layout: &ConsoleFrameLayout,
    font_system: &mut FontSystem,
) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    for (channel, area) in console_overlay_areas(geometry, layout, font_system) {
        let element = GfxElement::new_area_non_indexed_with_id(area, channel, channel);
        let leaf = tree.arena.new_node(element);
        tree.root.append(leaf, &mut tree.arena);
    }
    tree
}

/// Build a [`baumhard::gfx_structs::tree::MutatorTree`] that updates
/// an already-registered console overlay tree to the current
/// `(geometry, layout)` state
/// without rebuilding the arena. Pairs with
/// [`build_console_overlay_tree`] — both consume
/// [`console_overlay_areas`] so channels and slot counts match.
///
/// Use this for the keystroke hot path: input mutation moves only
/// the prompt line's text and cursor region; the borders /
/// scrollback / completion slots stay stable in shape, the
/// mutator overwrites their fields with the same values, and the
/// arena is reused. Open / close still use the full rebuild path
/// because the arena needs to be created or torn down. A change
/// in `scrollback_rows` or `completion_rows` (window resize)
/// shifts the structural signature and the dispatcher in
/// [`Renderer::rebuild_console_overlay_buffers`] falls back to a
/// rebuild.
pub(super) fn build_console_overlay_mutator(
    geometry: &ConsoleOverlayGeometry,
    layout: &ConsoleFrameLayout,
    font_system: &mut FontSystem,
) -> baumhard::gfx_structs::tree::MutatorTree<GfxMutator> {
    use baumhard::core::primitives::ApplyOperation;
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::gfx_structs::tree::MutatorTree;

    let mut mt = MutatorTree::new_with(GfxMutator::new_void(0));
    for (channel, area) in console_overlay_areas(geometry, layout, font_system) {
        let delta = DeltaGlyphArea::new(vec![
            GlyphAreaField::Text(area.text),
            GlyphAreaField::position(area.position.x.0, area.position.y.0),
            GlyphAreaField::bounds(area.render_bounds.x.0, area.render_bounds.y.0),
            GlyphAreaField::scale(area.scale.0),
            GlyphAreaField::line_height(area.line_height.0),
            GlyphAreaField::ColorFontRegions(area.regions),
            GlyphAreaField::Outline(area.outline),
            GlyphAreaField::Operation(ApplyOperation::Assign),
        ]);
        let mutator = GfxMutator::new(Mutation::AreaDelta(Box::new(delta)), channel);
        let id = mt.arena.new_node(mutator);
        mt.root.append(id, &mut mt.arena);
    }
    mt
}

/// Structural signature for the console overlay tree.
/// `(scrollback_rows, completion_rows)` from the layout. Two
/// calls share a signature iff the slot counts match — the
/// precondition for the in-place
/// [`build_console_overlay_mutator`] path. Window resize is the
/// only typical event that shifts these, so the signature stays
/// stable across keystroke / scrollback-grow / completion-update
/// frames and the §B2 path runs on those.
pub(super) fn console_overlay_signature(layout: &ConsoleFrameLayout) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    layout.scrollback_rows.hash(&mut h);
    layout.completion_rows.hash(&mut h);
    h.finish()
}
