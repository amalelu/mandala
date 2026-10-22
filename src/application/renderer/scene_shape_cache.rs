// SPDX-License-Identifier: MPL-2.0

//! Per-element re-shape cache shared by both scene passes — the
//! screen-space overlay sub-scene and the camera-transformed canvas
//! sub-scene.
//!
//! Both sub-scenes of
//! [`AppScene`](crate::application::scene_host::AppScene) are
//! re-walked after every change to any of their trees. Walking them
//! is cheap; *shaping* them is not — every walked `GlyphArea` used to
//! get a fresh `cosmic_text::Buffer`, a `set_rich_text` and a
//! `shape_until_scroll`, times `halos + 1`, under one `FONT_SYSTEM`
//! write guard, at mouse-move cadence. A picker hover changes one
//! cell's color; a console keystroke changes one line's text; an
//! edge-label drag moves one label. None of those is a reason to
//! re-shape the other fifty-odd elements, or the other seven canvas
//! roles.
//!
//! ## The reuse rule, and why it needs no notifications
//!
//! This cache does **not** subscribe to a dirty signal. It keeps,
//! per shaped element, a verbatim copy of every input the shaper
//! read, and re-validates against the live tree on each pass. An
//! element's output is reused only when all of the following still
//! hold at the same position in the walk:
//!
//! - the same tree ([`SceneTreeId`]),
//! - the same `GfxElement::unique_id`,
//! - the same registered tree offset (the walker adds it to every
//!   emitted buffer's `pos` and to the fill rect's `position`),
//! - a `GlyphArea` equal to the stored one — or, for an element that
//!   carries no area, still no area.
//!
//! That list is exactly what [`shape_one_element_into_buffers`]
//! reads: `element.glyph_area()`, `element.unique_id()`, and its
//! `offset` argument. Nothing else reaches the emitted buffers — or
//! the background fill, which
//! [`extract_background_rect`](super::tree_walker::extract_background_rect)
//! derives from `background_color`, `background_padding`,
//! `position`, `render_bounds`, `shape` and `zoom_visibility` of the
//! same area plus the same `unique_id` and `offset`.
//!
//! The consequence worth naming: because the check compares against
//! live state rather than trusting a producer to announce itself,
//! **a new writer of scene-tree state cannot make this cache go
//! stale by forgetting to notify it**. A mutator apply, a
//! `Scene::tree_mut` escape-hatch write, a full re-register — all of
//! them are seen the same way, as a `GlyphArea` that no longer
//! compares equal. That is what answers the canvas pass's ten
//! `flush_canvas_scene_buffers` call sites as a class rather than
//! one at a time: each of them mutates registered canvas trees and
//! then flushes, and whatever it wrote is what the next walk reads.
//! A caller that touched no tree at all re-shapes nothing; a caller
//! that re-registered one role re-shapes the elements of that role
//! whose shaping inputs actually moved.
//!
//! `GlyphArea`'s `PartialEq` is derived, so a field added to the
//! area joins the comparison by default rather than by anyone
//! remembering to wire it in. "By default" and not "always": the
//! derive is `derivative`'s, and a field carrying
//! `#[derivative(PartialEq = "ignore")]` opts back out — exactly
//! what `hitbox`, the one field skipped today, does. So the
//! ignore-list is pinned rather than trusted:
//! `test_glyph_area_equality_ignores_only_the_hitbox` destructures
//! `GlyphArea` exhaustively and asserts every other field breaks
//! `==`, which fails both on a new field nobody classified and on
//! an `ignore` added to an existing one. `hitbox` itself is on the
//! ignored side because the shaper never reads it.
//!
//! ## The one place `==` is not enough
//!
//! `GlyphArea`'s derived equality bottoms out, for the `regions`
//! field, in `BTreeSet` equality over `ColorFontRegion` — whose own
//! `Eq` is *set identity by range*, ignoring the font and color
//! pins. So `area_a == area_b` is true for two areas whose spans
//! are painted different colors, which is precisely what a
//! color-picker hover changes and nothing else. Comparing on `==`
//! alone would have reused every cell's buffers through a hover and
//! frozen the wheel's colors on screen. The reuse check therefore
//! adds
//! [`ColorFontRegions::same_content`](baumhard::core::primitives::ColorFontRegions::same_content),
//! the content-equality question that type now answers, on top of
//! `==`.
//!
//! The one input outside that set is the font database itself, which
//! `RegionFamilies::resolve` and cosmic-text's shaper both consult.
//! It is loaded once, by `fonts::load_fonts` behind a `lazy_static`,
//! and never mutated again — `db_mut` has exactly one call site in
//! the workspace — so it cannot invalidate a cached buffer at
//! runtime. A future runtime font load would have to clear this
//! cache, and this paragraph exists to tell whoever writes one.
//!
//! ## Draw order across a partial re-shape
//!
//! Each entry owns its own output — its buffers *and* its fill —
//! and entries sit in walk order, so re-shaping one element writes
//! back into the position that element already occupied. That is
//! what keeps the two order-sensitive draw streams stable while
//! only part of the scene re-shapes: `render.rs` pushes one quad per
//! fill into a single vertex stream, so a later fill paints over an
//! earlier one, and text areas are handed to glyphon in the same
//! order for the same reason.
//!
//! The mindmap's own keyed re-shape had to solve this the other way
//! round, because its fills live in one flat list keyed by element
//! rather than grouped by walk position: `reshape_buffer_for` drops
//! an element's fill and re-inserts it at the index the old one held
//! (see [`BackgroundRectSlot`](super::tree_buffers::BackgroundRectSlot)),
//! having first shipped the version that appended and floated the
//! element being edited above every other node's fill. Grouping by
//! walk position is that hazard's structural answer rather than its
//! bookkeeping one — there is no removal for an insertion point to
//! be lost across.
//!
//! Two functions carry that order to the render pass, and between
//! them they decide it for **all four** order-sensitive streams:
//! [`background_fills`] for the rect pipeline, and [`text_buffers`]
//! for each of the two glyphon passes. `text_buffers` carries a
//! second ordering besides the one between elements — the walker
//! emits an element's halo stamps before the glyph they ring, so
//! that the glyph draws on top — which makes its inner order
//! load-bearing too.
//!
//! Both are pinned **absolutely**, by
//! `test_the_draw_streams_come_out_in_walk_order` and
//! `test_the_text_stream_puts_every_halo_stamp_before_the_glyph_it_rings`,
//! and the reason they have to be — rather than the comparison a
//! reuse cache invites — is worth stating instead of rediscovering.
//! That comparison is "a partial pass produces what a full one
//! does", and
//! `test_canvas_partial_reshape_matches_a_full_one_stream_for_stream`
//! makes it; it catches a per-entry difference the in-place path
//! introduces. What it cannot catch is a reordering these two
//! functions apply *uniformly*, because it reads both of its sides
//! through them — a comparison between two runs of one reader can
//! only see what the reader is not doing to both. Before the two
//! absolute assertions existed, reversing either function left that
//! test and every other test in the workspace green; they are what
//! closed that, and the boundary is recorded here because it is a
//! property of the comparison's *shape* and will hold again for the
//! next assertion built that way.
//!
//! The reuse rule is pinned by
//! `test_scene_shape_cache_invalidates_on_every_writable_area_field`
//! and its neighbors in `renderer::tests`.

use glam::Vec2;

use baumhard::font::fonts;
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::scene::{Scene, SceneTreeId};

use super::tree_walker::shape_one_element_into_buffers;
use super::{MindMapTextBuffer, NodeBackgroundRect};

/// Which of the two sub-scenes a [`refresh`] pass is walking.
///
/// The passes differ in exactly two things, and both of them live
/// on this enum: whether the walker's background fills are kept,
/// and the site name the `FONT_SYSTEM` write guard is acquired
/// under. Everything else — the walk, the reuse rule, the
/// truncation — is one body serving both.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(crate) enum ScenePassKind {
    /// The camera-transformed sub-scene: borders, connections,
    /// portals, edge handles, connection labels, section frames and
    /// the two resize-handle roles.
    Canvas,
    /// The screen-space sub-scene: the console and the color picker.
    Overlay,
}

impl ScenePassKind {
    /// Whether this pass keeps the background fills the walker
    /// emits.
    ///
    /// The canvas pass does: its fills are drawn by the
    /// camera-transformed rect pipeline alongside the mindmap's own.
    /// The overlay pass does not, because there is no screen-space
    /// rect pipeline for them to reach — when a screen-space overlay
    /// actually needs fills, this arm starts keeping them and the
    /// palette pass reads
    /// [`background_fills`] the way the main pass already does.
    fn keeps_background_fills(self) -> bool {
        match self {
            ScenePassKind::Canvas => true,
            ScenePassKind::Overlay => false,
        }
    }

    /// The site name passed to
    /// [`fonts::acquire_font_system_write`], which reports it when a
    /// re-entrant same-thread acquire is caught. Distinct per pass
    /// so the report names which of the two was walking.
    fn font_system_site(self) -> &'static str {
        match self {
            ScenePassKind::Canvas => "scene_shape_cache::refresh (canvas)",
            ScenePassKind::Overlay => "scene_shape_cache::refresh (overlay)",
        }
    }
}

/// What one [`refresh`] pass did.
///
/// Returned rather than merely performed for the reason
/// `RebuildTier` on the app side is a value rather than a statement:
/// the pass runs inside a `Renderer`, and TEST_CONVENTIONS §T8 keeps
/// live wgpu out of the harness, so a granularity claim about it has
/// to be observable from somewhere a test can stand. `refresh` takes
/// a `&Scene` and not a renderer precisely so that place exists, and
/// these counts are what the claim is made in.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(crate) struct ScenePassCounts {
    /// Walk positions visited across every visible tree of the
    /// sub-scene — the number of elements the pass had to consider.
    pub(crate) walked: usize,
    /// How many of those were re-shaped because their shaping inputs
    /// no longer matched what the cached output was produced from.
    /// The rest kept the buffers and the fill they already had.
    pub(crate) reshaped: usize,
}

/// One scene element's shaped output plus the inputs it was shaped
/// from. Held in walk order by
/// [`Renderer::canvas_scene_elements`](super::Renderer#structfield.canvas_scene_elements)
/// and
/// [`Renderer::overlay_scene_elements`](super::Renderer#structfield.overlay_scene_elements);
/// see the module header for the reuse rule.
pub(crate) struct ShapedSceneElement {
    /// Which tree of the sub-scene the element came from.
    tree: SceneTreeId,
    /// The element's `unique_id`, one of the two things the shaper
    /// reads off the element.
    unique_id: usize,
    /// The registered tree offset in force when this was shaped.
    offset: Vec2,
    /// The area the output below was shaped from, cloned verbatim.
    /// `None` for a `Void` / `GlyphModel` element, which shapes to
    /// nothing but still occupies a walk position.
    area: Option<GlyphArea>,
    /// The shaped buffers, in emission order: halo stamps first,
    /// main glyph last.
    pub(super) buffers: Vec<MindMapTextBuffer>,
    /// The element's background fill, if it authored one and the
    /// pass keeps fills — see
    /// [`ScenePassKind::keeps_background_fills`]. An overlay-pass
    /// entry is always `None`.
    pub(super) background: Option<NodeBackgroundRect>,
}

impl ShapedSceneElement {
    /// Capture `element`'s shaping inputs alongside the output it
    /// produced.
    pub(super) fn new(
        tree: SceneTreeId,
        element: &GfxElement,
        offset: Vec2,
        buffers: Vec<MindMapTextBuffer>,
        background: Option<NodeBackgroundRect>,
    ) -> Self {
        ShapedSceneElement {
            tree,
            unique_id: element.unique_id(),
            offset,
            area: element.glyph_area().cloned(),
            buffers,
            background,
        }
    }

    /// Whether the output held here is still what shaping `element`
    /// — at walk position `tree` / `offset` — would produce. See the
    /// module header for what this set is and why it is complete.
    ///
    /// # Costs
    ///
    /// Two integer compares, a `Vec2` compare, one `GlyphArea`
    /// equality and one region-content walk — together
    /// O(text length + region count) and allocation-free. Cheap
    /// against the cosmic-text shaping it stands in for.
    pub(super) fn still_matches(&self, tree: SceneTreeId, element: &GfxElement, offset: Vec2) -> bool {
        if self.tree != tree || self.unique_id != element.unique_id() || self.offset != offset {
            return false;
        }
        match (self.area.as_ref(), element.glyph_area()) {
            (None, None) => true,
            // `==` covers every area field; `same_content` covers
            // what `==` cannot see inside `regions` — see the module
            // header's "The one place `==` is not enough".
            (Some(cached), Some(live)) => cached == live && cached.regions.same_content(&live.regions),
            _ => false,
        }
    }
}

/// Bring `shaped` back in step with `scene`, re-shaping only the
/// walk positions whose inputs moved, and report what that took.
///
/// `ids` is the sub-scene's tree handles in layer order; the walk
/// visits every descendant of every *visible* one, in that order, so
/// entry `i` of `shaped` describes walk position `i`. A position
/// whose live element still matches the entry sitting there keeps
/// its buffers and its fill untouched; every other position is
/// re-shaped in place. Positions past the end of the walk belong to
/// a tree that has since been unregistered or shortened and are
/// dropped.
///
/// # Costs
///
/// O(sum of descendants) across every tree named in `ids` for the
/// walk and the equality checks, plus one `cosmic_text::Buffer`
/// allocation and shaping per *changed* non-empty `GlyphArea`
/// (times `halos + 1`). The `FONT_SYSTEM` write guard is acquired
/// lazily on the first element that actually needs shaping, so a
/// pass in which nothing moved takes no lock at all — which is the
/// steady state for every sub-scene tree a given caller did not
/// touch. Empty sub-scenes short-circuit cheaply.
pub(crate) fn refresh(
    scene: &Scene,
    ids: &[SceneTreeId],
    shaped: &mut Vec<ShapedSceneElement>,
    kind: ScenePassKind,
) -> ScenePassCounts {
    let keep_fills = kind.keeps_background_fills();
    // Lazily acquired: a pass in which every element still matches
    // its cached inputs never touches the lock.
    let mut font_system: Option<fonts::FontSystemWriteGuard> = None;
    let mut counts = ScenePassCounts {
        walked: 0,
        reshaped: 0,
    };
    for &id in ids {
        let Some(entry) = scene.get(id) else {
            continue;
        };
        if !entry.visible() {
            continue;
        }
        let tree = entry.tree();
        let offset = entry.offset();
        for descendant_id in tree.root().descendants(&tree.arena) {
            let Some(element) = tree.arena.get(descendant_id).map(|n| n.get()) else {
                continue;
            };
            let slot = counts.walked;
            counts.walked += 1;
            if shaped
                .get(slot)
                .is_some_and(|cached| cached.still_matches(id, element, offset))
            {
                continue;
            }
            let font_system =
                font_system.get_or_insert_with(|| fonts::acquire_font_system_write(kind.font_system_site()));
            let mut buffers = Vec::new();
            let mut background = None;
            shape_one_element_into_buffers(
                element,
                offset,
                font_system,
                &mut |_unique_id, buffer| buffers.push(buffer),
                &mut |rect| {
                    if keep_fills {
                        background = Some(rect);
                    }
                },
            );
            let fresh = ShapedSceneElement::new(id, element, offset, buffers, background);
            match shaped.get_mut(slot) {
                Some(existing) => *existing = fresh,
                None => shaped.push(fresh),
            }
            counts.reshaped += 1;
        }
    }
    // Anything past the last walked position belongs to a tree that
    // has since been unregistered or shortened.
    shaped.truncate(counts.walked);
    counts
}

/// Every shaped text buffer of a pass's output, in draw order.
///
/// The single reader of the grouping for the render path, so the
/// main pass and the palette pass flatten it the same way and a
/// test can assert on the same sequence the GPU is handed.
pub(super) fn text_buffers(shaped: &[ShapedSceneElement]) -> impl Iterator<Item = &MindMapTextBuffer> {
    shaped.iter().flat_map(|element| element.buffers.iter())
}

/// How many buffers [`text_buffers`] will yield — the capacity the
/// render pass reserves before flattening, counted without walking
/// into the groups.
pub(super) fn buffer_count(shaped: &[ShapedSceneElement]) -> usize {
    shaped.iter().map(|element| element.buffers.len()).sum()
}

/// Every background fill of a pass's output, in draw order —
/// [`text_buffers`]'s sibling for the rect pipeline. Empty for an
/// overlay pass, which keeps no fills.
pub(super) fn background_fills(shaped: &[ShapedSceneElement]) -> impl Iterator<Item = &NodeBackgroundRect> {
    shaped.iter().filter_map(|element| element.background.as_ref())
}
