// SPDX-License-Identifier: MPL-2.0

//! Click-target handlers for the native event loop: default click,
//! reparent-target click, connect-target click, plus the
//! mode-aware variant of `rebuild_all`. WASM is gated out at the
//! parent module's `#[cfg]`.

#![cfg(not(target_arch = "wasm32"))]

use baumhard::mindmap::tree_builder::PortalPart;

use super::click_triggers::fire_onclick_triggers;
use super::dispatch::compute_node_click_selection;
#[cfg(not(target_arch = "wasm32"))]
use super::input_context::InputHandlerContext;
use super::scene_rebuild::{build_overlaid_tree, rebuild_scene_only, RebuildTier};
use super::{now_ms, InteractionMode};
use crate::application::document::{
    MindMapDocument, SelectionState, REPARENT_SOURCE_COLOR, REPARENT_TARGET_COLOR,
};
use crate::application::renderer::Renderer;

/// The renderer-free half of a click's context: everything
/// [`handle_click_core`] reads or mutates before it names the
/// canvas work the outcome owes.
///
/// `app_scene` is here rather than on the shell because the portal
/// hit-test is part of the *selection* decision, not part of the
/// rebuild — and `AppScene` holds no GPU state, so it stands up in
/// a test harness (TEST_CONVENTIONS §T8 excludes wgpu, not the
/// scene host).
#[cfg(not(target_arch = "wasm32"))]
pub(super) struct ClickCore<'a> {
    pub document: &'a mut MindMapDocument,
    pub mindmap_tree: &'a mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    pub app_scene: &'a mut crate::application::scene_host::AppScene,
    pub scene_cache: &'a mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    pub interaction_mode: &'a InteractionMode,
}

/// Fire the click's `OnClick` triggers, resolve the new selection,
/// and name the rebuild tier the outcome owes — all without a
/// renderer.
///
/// When the node hit test misses, falls through to portal-marker
/// hit-testing and then edge hit testing so the user can click on a
/// connection path to select it. If the clicked node has an
/// `OnClick` trigger binding, the bound custom mutation fires (both
/// node mutations and any document actions) before the selection
/// update, so document actions (theme switches etc.) take effect
/// before the rebuild picks up the new state.
///
/// The two renderer-derived inputs arrive as plain values:
/// `canvas_pos` is the press point already through
/// `Renderer::screen_to_canvas`, and `canvas_per_pixel` is
/// `Renderer::canvas_per_pixel` — the scaling `EDGE_HIT_TOLERANCE_PX`
/// needs is applied once, inside the shared last rung
/// [`dispatch::edge_under_pointer`](super::dispatch::edge_under_pointer),
/// rather than at each of its three callers. Both are pure camera math, which is
/// why this split leaves nothing renderer-shaped behind — the same
/// shape `ReleaseCommit` / `ReleaseRefresh` gave the drag-release
/// path, and for the same reason: it makes "which tier does this
/// interaction ask for?" a question the harness can ask.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn handle_click_core(
    hit: Option<String>,
    hit_section: Option<usize>,
    canvas_pos: glam::Vec2,
    canvas_per_pixel: super::dispatch::CanvasPerPixel,
    shift_pressed: bool,
    ctx: ClickCore<'_>,
) -> RebuildTier {
    let ClickCore {
        document: doc,
        mindmap_tree,
        app_scene,
        scene_cache,
        interaction_mode,
    } = ctx;

    // OnClick triggers fire before the selection update so that
    // document actions (theme switches etc.) take effect before
    // the scene rebuild picks up the new state.
    let triggers_fired = match hit.as_ref() {
        Some(id) => fire_onclick_triggers(
            doc,
            mindmap_tree,
            scene_cache,
            id,
            hit_section,
            super::PLATFORM_CONTEXT,
            now_ms() as u64,
        ),
        None => false,
    };

    // Snapshot before the write: the tier is a function of how the
    // selection *moved*, so a post-write capture would read the new
    // value back and answer `SceneOnly` for every transition.
    let prev_selection = doc.selection.clone();

    // Update selection state
    match (&hit, shift_pressed) {
        (Some(id), shift) => {
            doc.selection =
                compute_node_click_selection(&doc.selection, id, hit_section, shift, interaction_mode);
        }
        (None, false) => {
            // Node miss — fall through: first try portal markers
            // (label glyphs attached to their endpoint nodes),
            // then edge hit testing, then finally deselect. A
            // portal-marker click selects the specific label
            // via `SelectionState::PortalLabel { .. }` so wheel
            // / copy / paste / cut / drag all operate on just
            // that endpoint's state; double-click is handled
            // separately by the event loop and pans the camera
            // to the opposite endpoint.
            //
            // One BVH descent over the portal tree names both the
            // endpoint and which of its two sibling leaves — icon
            // or text — the click landed on, so the sub-part
            // precedence is decided by geometry (smallest area
            // wins) rather than by the order two side maps happen
            // to be scanned in.
            if let Some(hit) = app_scene.portal_at(canvas_pos) {
                let sel = crate::application::document::PortalLabelSel {
                    edge_key: hit.edge_key,
                    endpoint_node_id: hit.endpoint_node_id,
                };
                doc.selection = match hit.part {
                    PortalPart::Text => SelectionState::PortalText(sel),
                    PortalPart::Icon => SelectionState::PortalLabel(sel),
                };
            } else {
                // The same last rung the browser's click release and
                // the touch tap run, through the one body all three
                // share.
                doc.selection =
                    super::dispatch::edge_under_pointer(canvas_pos, &doc.mindmap, canvas_per_pixel);
            }
        }
        (None, true) => {
            // Shift+click on empty space: keep current selection (no edge
            // hit test — shift is reserved for multi-node).
        }
    }

    RebuildTier::for_click(triggers_fired, &prev_selection, &doc.selection)
}

/// Handle a click event: run [`handle_click_core`], then perform the
/// rebuild tier it named.
///
/// Pre-#37 this ran `rebuild_all` for every click outcome, including
/// an edge-label → edge-label selection change that cannot touch a
/// node text buffer. `rebuild_after_selection_change` had existed for
/// exactly this since before the browser release path started using
/// it — CODE_CONVENTIONS §4 makes the two targets peers, and the
/// browser was the better-optimized one here.
///
/// The pointer position and the Shift state are read from `ctx`
/// rather than passed beside it, so they cannot disagree with it.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn handle_click(
    hit: Option<String>,
    hit_section: Option<usize>,
    ctx: &mut InputHandlerContext<'_>,
) {
    let doc = match ctx.document.as_mut() {
        Some(d) => d,
        None => return,
    };
    let canvas_pos = ctx
        .renderer
        .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
    let tier = handle_click_core(
        hit,
        hit_section,
        canvas_pos,
        super::dispatch::CanvasPerPixel::of(ctx.renderer),
        ctx.modifiers.shift_key(),
        ClickCore {
            document: doc,
            mindmap_tree: ctx.mindmap_tree,
            app_scene: ctx.app_scene,
            scene_cache: ctx.scene_cache,
            interaction_mode: ctx.interaction_mode,
        },
    );
    tier.execute(&mut super::dispatch::cross_dispatch::rebuild_ctx!(ctx, doc));
}

/// Rebuild tree, connections, and borders like `rebuild_all`, but additionally
/// overlays reparent-mode highlights on top of the normal selection highlight.
/// `hovered_node` is the node currently under the cursor (highlighted green as
/// the drop target) when in reparent mode; it is ignored in Normal mode.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn rebuild_all_with_mode(
    doc: &MindMapDocument,
    interaction_mode: &InteractionMode,
    hovered_node: Option<&str>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    // Build a single flat list of (mind_node_id, color) pairs that
    // `apply_tree_highlights` applies via baumhard's mutator/walker.
    // Order matters: later entries override earlier ones via the
    // repeated `SetRegionColor` mutation, so selection (cyan) is
    // listed first, then mode-specific source (orange), then the
    // hovered target (green). This matches the previous behavior
    // where reparent_source_highlight was documented to override
    // selection_highlight on conflict.
    // Highlight tuples are `(node_id, section_idx?, color)`. A
    // Section / MultiSection narrow the highlight to the
    // selected sections only; mode-driven Reparent / Connect
    // highlights always paint every section (the gesture is
    // whole-node). Routes through the canonical
    // `highlight_entries_for` helper so all four node-tree
    // rebuild sites (here, `rebuild_all`,
    // `rebuild_selection_highlight`, and the rubber-band drain
    // through it) share one mapping — including its rule for
    // which of the selection and the rubber-band preview wins.
    let mut highlights = super::scene_rebuild::highlight_entries_for(doc);
    match interaction_mode {
        InteractionMode::Reparent { sources } => {
            for s in sources {
                highlights.push((s.as_str(), None, REPARENT_SOURCE_COLOR));
            }
            if let Some(h) = hovered_node {
                if !sources.iter().any(|s| s == h) {
                    highlights.push((h, None, REPARENT_TARGET_COLOR));
                }
            }
        }
        InteractionMode::Connect { source } => {
            highlights.push((source.as_str(), None, REPARENT_SOURCE_COLOR));
            if let Some(h) = hovered_node {
                if h != source {
                    highlights.push((h, None, REPARENT_TARGET_COLOR));
                }
            }
        }
        // Default / NodeEdit / Resize don't contribute selection-
        // tinting highlights. NodeEdit dimming is a separate overlay
        // `build_overlaid_tree` stamps; Resize tinting rides on the
        // handle trees.
        InteractionMode::Default | InteractionMode::NodeEdit { .. } | InteractionMode::Resize { .. } => {}
    }
    let new_tree = build_overlaid_tree(doc, interaction_mode, highlights);
    renderer.rebuild_buffers_from_tree(&new_tree.tree);

    rebuild_scene_only(doc, interaction_mode, app_scene, renderer, scene_cache);
    renderer.set_mode_status_text(super::scene_rebuild::mode_status_line(interaction_mode, doc));

    *mindmap_tree = Some(new_tree);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::{EdgeRef, GraphemeRange, SectionSel, SectionSpan};
    use crate::application::scene_host::AppScene;
    use baumhard::mindmap::scene_cache::SceneConnectionCache;

    /// Everything a click core needs besides the hit, owned by the
    /// test so the borrows in [`ClickCore`] have somewhere to point.
    struct CoreWorld {
        doc: MindMapDocument,
        mindmap_tree: Option<baumhard::mindmap::tree_builder::MindMapTree>,
        app_scene: AppScene,
        scene_cache: SceneConnectionCache,
        interaction_mode: InteractionMode,
    }

    impl CoreWorld {
        fn new() -> Self {
            Self {
                doc: load_test_doc(),
                mindmap_tree: None,
                app_scene: AppScene::new(),
                scene_cache: SceneConnectionCache::default(),
                interaction_mode: InteractionMode::Default,
            }
        }

        fn core(&mut self) -> ClickCore<'_> {
            ClickCore {
                document: &mut self.doc,
                mindmap_tree: &mut self.mindmap_tree,
                app_scene: &mut self.app_scene,
                scene_cache: &mut self.scene_cache,
                interaction_mode: &self.interaction_mode,
            }
        }
    }

    /// Far outside the testament map's extent, so the edge hit-test
    /// misses every connection at the tolerance below and the click
    /// resolves to empty canvas. A coordinate inside the map would
    /// make the assertions depend on which edge happens to sit there.
    const FAR_OFF_CANVAS: glam::Vec2 = glam::Vec2::new(1.0e6, 1.0e6);

    /// What a 1:1 camera reports for `canvas_per_pixel()`, which is
    /// what the core scales `EDGE_HIT_TOLERANCE_PX` by.
    fn unzoomed() -> crate::application::app::dispatch::CanvasPerPixel {
        crate::application::app::dispatch::CanvasPerPixel::from_ratio(1.0)
    }

    fn first_node_id(doc: &MindMapDocument) -> String {
        doc.mindmap
            .nodes
            .keys()
            .min()
            .cloned()
            .expect("the testament fixture has nodes")
    }

    /// A click whose only outcome is a selection change between two
    /// selections that live outside the node tree asks for
    /// `SceneOnly`, not `All`.
    ///
    /// This is the whole of #37 item 1: `handle_click` ended with an
    /// unconditional `rebuild_all`, so this click paid for a
    /// `doc.build_tree()` plus a cosmic-text buffer rebuild it could
    /// not have invalidated.
    ///
    /// Fails on the input below the moment the core stops consulting
    /// the selection delta — the pre-fix shape answers `All` here.
    /// The second half is the control that keeps the first from
    /// being "answer `SceneOnly` always": the *same* empty-canvas
    /// click from a node selection still has a highlight to clear
    /// out of a text buffer, and still asks for `All`.
    #[test]
    fn test_click_core_selection_only_change_asks_for_the_scene_tier() {
        let mut world = CoreWorld::new();
        world.doc.selection = SelectionState::Edge(EdgeRef::new("a", "b", "cross_link"));
        let tier = handle_click_core(None, None, FAR_OFF_CANVAS, unzoomed(), false, world.core());
        assert!(
            matches!(world.doc.selection, SelectionState::None),
            "precondition: the click must land on empty canvas, or this pins the wrong \
             transition — got {:?}",
            world.doc.selection
        );
        assert_eq!(
            tier,
            RebuildTier::SceneOnly,
            "an Edge -> None click touches no node text buffer"
        );

        let mut from_node = CoreWorld::new();
        let node_id = first_node_id(&from_node.doc);
        from_node.doc.selection = SelectionState::Single(node_id);
        let tier = handle_click_core(None, None, FAR_OFF_CANVAS, unzoomed(), false, from_node.core());
        assert_eq!(
            tier,
            RebuildTier::All,
            "the same click from a node selection still has a highlight to clear"
        );
    }

    /// A click that lands on a node asks for `All` — the node's
    /// highlight has to be stamped into its text buffer.
    ///
    /// Fails if the core ever answers the cheap tier for a node hit,
    /// which would leave the clicked node unhighlighted.
    #[test]
    fn test_click_core_on_a_node_asks_for_the_full_tier() {
        let mut world = CoreWorld::new();
        let node_id = first_node_id(&world.doc);
        world.doc.selection = SelectionState::None;
        let tier = handle_click_core(
            Some(node_id.clone()),
            None,
            FAR_OFF_CANVAS,
            unzoomed(),
            false,
            world.core(),
        );
        assert!(
            matches!(&world.doc.selection, SelectionState::Single(id) if *id == node_id),
            "precondition: the hit must have produced the node selection — got {:?}",
            world.doc.selection
        );
        assert_eq!(tier, RebuildTier::All);
    }

    /// Two distinct node ids from the fixture, smallest first.
    fn two_node_ids(doc: &MindMapDocument) -> (String, String) {
        let mut ids: Vec<&String> = doc.mindmap.nodes.keys().collect();
        ids.sort();
        assert!(ids.len() >= 2, "the testament fixture has at least two nodes");
        (ids[0].clone(), ids[1].clone())
    }

    /// Shift on empty canvas keeps the selection; the same click
    /// without Shift clears it.
    ///
    /// `handle_click` reads Shift from `ctx.modifiers` and the shell
    /// cannot run without a live `Renderer` (TEST_CONVENTIONS §T8),
    /// so this pins the flag at the core boundary the shell feeds.
    /// Fails if the core stops honoring `shift_pressed` on a miss,
    /// or if the two branches collapse into one.
    #[test]
    fn test_click_core_shift_on_empty_canvas_keeps_the_selection() {
        let mut world = CoreWorld::new();
        let node_id = first_node_id(&world.doc);
        world.doc.selection = SelectionState::Single(node_id.clone());
        handle_click_core(None, None, FAR_OFF_CANVAS, unzoomed(), true, world.core());
        assert!(
            matches!(&world.doc.selection, SelectionState::Single(id) if *id == node_id),
            "Shift+click on empty canvas must keep the selection, got {:?}",
            world.doc.selection
        );

        let mut control = CoreWorld::new();
        control.doc.selection = SelectionState::Single(node_id);
        handle_click_core(None, None, FAR_OFF_CANVAS, unzoomed(), false, control.core());
        assert!(
            matches!(control.doc.selection, SelectionState::None),
            "the same click without Shift must clear it, got {:?}",
            control.doc.selection
        );
    }

    /// Shift on a node adds it to the selection; without Shift it
    /// replaces the selection. Fails if `shift_pressed` stops
    /// reaching `compute_node_click_selection` through the core.
    #[test]
    fn test_click_core_shift_on_a_node_adds_it_to_the_selection() {
        let mut world = CoreWorld::new();
        let (first, second) = two_node_ids(&world.doc);
        world.doc.selection = SelectionState::Single(first.clone());
        let tier = handle_click_core(
            Some(second.clone()),
            None,
            FAR_OFF_CANVAS,
            unzoomed(),
            true,
            world.core(),
        );
        match &world.doc.selection {
            SelectionState::Multi(ids) => {
                assert!(
                    ids.contains(&first) && ids.contains(&second) && ids.len() == 2,
                    "Shift+click must add the node to the existing one, got {ids:?}"
                );
            }
            other => panic!("Shift+click on a second node must produce Multi, got {other:?}"),
        }
        assert_eq!(
            tier,
            RebuildTier::All,
            "a node selection change repaints node highlights"
        );

        let mut control = CoreWorld::new();
        control.doc.selection = SelectionState::Single(first);
        handle_click_core(
            Some(second.clone()),
            None,
            FAR_OFF_CANVAS,
            unzoomed(),
            false,
            control.core(),
        );
        assert!(
            matches!(&control.doc.selection, SelectionState::Single(id) if *id == second),
            "the same click without Shift must replace the selection, got {:?}",
            control.doc.selection
        );
    }

    /// Why `RebuildTier::for_click`'s `triggers_fired` flag cannot
    /// be observed from this path today, stated as a test rather
    /// than as a comment nobody re-checks.
    ///
    /// `fire_onclick_triggers` runs only for a node hit, and every
    /// selection a node hit can produce — `Single`, `Section`,
    /// `Multi`, `MultiSection`, or the `None` a shift-toggle-off
    /// leaves behind a node-ish `prev` — already answers `All` on
    /// the selection delta alone. So the flag is redundant *here*,
    /// and this test is what will notice when it stops being: a
    /// future `OnClick` on an edge or a portal marker would fire a
    /// trigger from an edge-adjacent selection, and the flag is the
    /// only thing that would still ask for the full rebuild.
    ///
    /// Fails when a node-hit click starts resolving to a selection
    /// outside the node-ish set, which is exactly the change that
    /// would make the flag load-bearing.
    #[test]
    fn test_every_click_that_can_fire_a_trigger_already_needs_the_full_tier() {
        let doc = load_test_doc();
        let node_id = first_node_id(&doc);
        // Every starting selection the shift and non-shift branches
        // of `compute_node_click_selection` route differently.
        let starts = [
            SelectionState::None,
            SelectionState::Single(node_id.clone()),
            SelectionState::Single("other".to_string()),
            SelectionState::Multi(vec![node_id.clone(), "other".to_string()]),
            SelectionState::Edge(EdgeRef::new("a", "b", "cross_link")),
            SelectionState::Section(SectionSel {
                node_id: node_id.clone(),
                section_idx: 0,
            }),
        ];
        for start in &starts {
            for shift in [false, true] {
                for section in [None, Some(0usize)] {
                    let new = compute_node_click_selection(
                        start,
                        &node_id,
                        section,
                        shift,
                        &InteractionMode::NodeEdit {
                            node_id: node_id.clone(),
                        },
                    );
                    assert_eq!(
                        RebuildTier::for_selection_change(start, &new),
                        RebuildTier::All,
                        "{:?} -> {:?} (shift={shift}, section={section:?}) is a node-hit outcome, \
                         so it must already need the full tier",
                        start,
                        new
                    );
                }
            }
        }
    }

    fn node_edit_for(id: &str) -> InteractionMode {
        InteractionMode::NodeEdit {
            node_id: id.to_string(),
        }
    }

    fn sec(node_id: &str, idx: usize) -> SectionSel {
        SectionSel {
            node_id: node_id.to_string(),
            section_idx: idx,
        }
    }

    // Plain click — section routing rules.

    #[test]
    fn test_plain_click_multi_section_in_node_edit_routes_to_section() {
        let result =
            compute_node_click_selection(&SelectionState::None, "n0", Some(2), false, &node_edit_for("n0"));
        match result {
            SelectionState::Section(s) => assert_eq!(s, sec("n0", 2)),
            other => panic!("expected Section(n0,2), got {other:?}"),
        }
    }

    #[test]
    fn test_plain_click_multi_section_in_default_mode_folds_to_single() {
        let result = compute_node_click_selection(
            &SelectionState::None,
            "n0",
            Some(2),
            false,
            &InteractionMode::Default,
        );
        match result {
            SelectionState::Single(id) => assert_eq!(id, "n0"),
            other => panic!("expected Single(n0), got {other:?}"),
        }
    }

    #[test]
    fn test_plain_click_multi_section_in_node_edit_on_other_node_folds_to_single() {
        let result =
            compute_node_click_selection(&SelectionState::None, "n0", Some(2), false, &node_edit_for("n1"));
        match result {
            SelectionState::Single(id) => assert_eq!(id, "n0"),
            other => panic!("expected Single(n0), got {other:?}"),
        }
    }

    #[test]
    fn test_plain_click_no_section_in_node_edit_returns_single() {
        // hit_section = None → always Single regardless of mode.
        let result =
            compute_node_click_selection(&SelectionState::None, "n0", None, false, &node_edit_for("n0"));
        match result {
            SelectionState::Single(id) => assert_eq!(id, "n0"),
            other => panic!("expected Single(n0), got {other:?}"),
        }
    }

    // Shift+click — section routing rules.

    #[test]
    fn test_shift_click_same_section_in_node_edit_toggles_off() {
        let result = compute_node_click_selection(
            &SelectionState::Section(sec("n0", 1)),
            "n0",
            Some(1),
            true,
            &node_edit_for("n0"),
        );
        assert!(matches!(result, SelectionState::None), "got {result:?}");
    }

    #[test]
    fn test_shift_click_different_section_in_node_edit_promotes_to_multi_section() {
        let result = compute_node_click_selection(
            &SelectionState::Section(sec("n0", 0)),
            "n0",
            Some(1),
            true,
            &node_edit_for("n0"),
        );
        match result {
            SelectionState::MultiSection(secs) => {
                assert_eq!(secs, vec![sec("n0", 0), sec("n0", 1)]);
            }
            other => panic!("expected MultiSection, got {other:?}"),
        }
    }

    #[test]
    fn test_shift_click_section_outside_node_edit_falls_back_to_node_path() {
        // Default mode + hit_section=Some → folds to whole-node shift+click.
        // Starting from None: result is fresh Single.
        let result = compute_node_click_selection(
            &SelectionState::None,
            "n0",
            Some(1),
            true,
            &InteractionMode::Default,
        );
        match result {
            SelectionState::Single(id) => assert_eq!(id, "n0"),
            other => panic!("expected Single(n0), got {other:?}"),
        }
    }

    #[test]
    fn test_shift_click_multi_section_remove_narrows_to_single_section() {
        let prev = SelectionState::MultiSection(vec![sec("n0", 0), sec("n0", 1)]);
        let result = compute_node_click_selection(&prev, "n0", Some(1), true, &node_edit_for("n0"));
        match result {
            SelectionState::Section(s) => assert_eq!(s, sec("n0", 0)),
            other => panic!("expected Section(n0,0), got {other:?}"),
        }
    }

    /// Cross-node MultiSection: starting from a `MultiSection` set
    /// containing sections of node A, shift-clicking a section of
    /// node B (while in `NodeEdit { B }`) extends the set with the
    /// new (node_id, section_idx) pair. The dedup-by-(node_id,
    /// section_idx) identity is the load-bearing invariant the
    /// docstring on `compute_node_click_selection` calls out.
    #[test]
    fn test_shift_click_extends_multi_section_across_distinct_nodes() {
        let prev = SelectionState::MultiSection(vec![sec("a", 0), sec("a", 1)]);
        let result = compute_node_click_selection(&prev, "b", Some(0), true, &node_edit_for("b"));
        match result {
            SelectionState::MultiSection(secs) => {
                assert_eq!(secs.len(), 3, "got {secs:?}");
                assert!(secs.contains(&sec("a", 0)));
                assert!(secs.contains(&sec("a", 1)));
                assert!(secs.contains(&sec("b", 0)));
            }
            other => panic!("expected MultiSection of length 3, got {other:?}"),
        }
    }

    /// `SectionRange` as the *starting* state: shift+click on a
    /// node folds to fresh `Single` (the node-path takes the
    /// non-section branch in the match arm). Pins the explicit
    /// `SectionRange` arm in `compute_node_click_selection`.
    #[test]
    fn test_shift_click_node_from_section_range_collapses_to_single() {
        let prev = SelectionState::SectionRange {
            sel: sec("n0", 0),
            section_span: SectionSpan::single(0),
            grapheme_range: GraphemeRange::new(1, 3),
        };
        let result = compute_node_click_selection(&prev, "n1", None, true, &InteractionMode::Default);
        match result {
            SelectionState::Single(id) => assert_eq!(id, "n1"),
            other => panic!("expected Single(n1), got {other:?}"),
        }
    }

    /// Cross-node MultiSection toggle-off: shift-clicking a section
    /// already in the set removes only that pair, leaving the
    /// other-node members alone.
    #[test]
    fn test_shift_click_removes_cross_node_section_from_multi_section() {
        let prev = SelectionState::MultiSection(vec![sec("a", 0), sec("a", 1), sec("b", 0)]);
        let result = compute_node_click_selection(&prev, "b", Some(0), true, &node_edit_for("b"));
        match result {
            SelectionState::MultiSection(secs) => {
                assert_eq!(secs.len(), 2, "got {secs:?}");
                assert!(secs.contains(&sec("a", 0)));
                assert!(secs.contains(&sec("a", 1)));
                assert!(!secs.contains(&sec("b", 0)));
            }
            other => panic!("expected MultiSection of length 2, got {other:?}"),
        }
    }

    // Plain click — non-section behavior stays intact.

    #[test]
    fn test_plain_click_overrides_existing_multi_with_single() {
        let prev = SelectionState::Multi(vec!["a".into(), "b".into()]);
        let result = compute_node_click_selection(&prev, "n0", None, false, &InteractionMode::Default);
        match result {
            SelectionState::Single(id) => assert_eq!(id, "n0"),
            other => panic!("expected Single(n0), got {other:?}"),
        }
    }

    // Shift+click — whole-node toggle behavior stays intact.

    #[test]
    fn test_shift_click_same_single_node_toggles_off() {
        let result = compute_node_click_selection(
            &SelectionState::Single("n0".into()),
            "n0",
            None,
            true,
            &InteractionMode::Default,
        );
        assert!(matches!(result, SelectionState::None), "got {result:?}");
    }

    #[test]
    fn test_shift_click_different_single_node_promotes_to_multi() {
        let result = compute_node_click_selection(
            &SelectionState::Single("a".into()),
            "b",
            None,
            true,
            &InteractionMode::Default,
        );
        match result {
            SelectionState::Multi(ids) => {
                assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
            }
            other => panic!("expected Multi, got {other:?}"),
        }
    }
}
