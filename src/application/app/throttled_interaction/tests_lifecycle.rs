// SPDX-License-Identifier: MPL-2.0

//! Differential oracle over the throttled-drag lifecycle.
//!
//! Every scenario below scripts one gesture — press, N cursor
//! samples, zero or more drains, release — against the renderer-free
//! commit core and renders one trace line per step describing
//! everything the step could observably change: the pending and
//! total accumulator state, the committed model value the gesture
//! owns, the undo-stack depth, the scene-cache entry count, and (on
//! the release step) the canvas decree the commit hands back.
//!
//! The expected traces are the oracle. They were authored against
//! the seven hand-written release arms in `event_mouse_click.rs` and
//! the five-plus-two accumulate arms in `event_cursor_moved.rs`, and
//! they are carried across the unification byte for byte — `git
//! log -p` on this file shows the driver changing and not one
//! expected line moving.
//!
//! **One expected column has moved since**, in the four `MovingNode`
//! releases: its commit core now *takes* the pending slot instead of
//! peeking it, like the other six cores, so the post-release `pend=`
//! reads `(0,0)` rather than the stale value. Nothing else on those
//! lines changed — same model, same tree, same undo depth, same
//! cache, same decree. The stale value was only ever observable
//! here, because production drops the interaction at release.
//!
//! **What a `Drain` step models.** The real `drain()` bodies need a
//! live `Renderer`, which TEST_CONVENTIONS §T8 keeps out of the
//! harness. A `Drain` step here performs exactly the part of the
//! real drain that the *release* path can observe, and nothing else:
//!
//! - `MovingNode` — folds `pending_delta` into the tree, clears
//!   pending. The release path reads both (it re-flushes any
//!   surviving pending into the same tree and clears the scene cache
//!   only when there was some).
//! - `MovingSection` / `NodeResize` / `SectionResize` — tree-only
//!   writes the release path never reads back; it recomputes from
//!   `start_*` plus `total_delta`. Only the pending clear is
//!   modeled.
//! - `EdgeHandle` — writes the edge from `total_delta`, clears
//!   pending. The release re-applies from the same `total_delta`, so
//!   the trace shows the intermediate write landing and then being
//!   idempotently repeated.
//! - `PortalLabel` / `EdgeLabel` — writes the edge from the latched
//!   cursor and takes it. The release commits nothing further when
//!   the latch is empty, which is exactly the case this pins.

use glam::Vec2;

use baumhard::mindmap::scene_cache::{EdgeKey, SampleParams, SceneConnectionCache};
use baumhard::mindmap::tree_builder::{MindMapTree, ResizeHandleSide};

use crate::application::document::{apply_drag_delta, EdgeRef, MindMapDocument};

use super::pending::DragInput;
use super::release::{ReleaseCommit, ReleaseRefresh};
use super::ThrottledDragInteraction;

/// One scripted lifecycle step.
#[derive(Debug, Clone, Copy)]
enum Step {
    /// One `CursorMoved` sample: `delta` is the canvas-space
    /// movement since the previous event, `cursor` the absolute
    /// canvas-space position now. Both are handed to the gesture;
    /// its pending discipline decides which half it keeps.
    Move { delta: Vec2, cursor: Vec2 },
    /// A drain landed — see the module header for exactly what
    /// each variant's drain is modeled as doing.
    Drain,
    /// The button came up.
    Release,
}

/// Shorthand for a `Move` step.
fn mv(dx: f32, dy: f32, cx: f32, cy: f32) -> Step {
    Step::Move {
        delta: Vec2::new(dx, dy),
        cursor: Vec2::new(cx, cy),
    }
}

/// The mutable world one scenario runs against. `document` is an
/// `Option` because two of the seven release bodies do work before
/// they check whether a document is loaded, and the `None` column is
/// part of what the oracle pins.
struct World {
    document: Option<MindMapDocument>,
    mindmap_tree: Option<MindMapTree>,
    scene_cache: SceneConnectionCache,
}

impl World {
    fn commit(&mut self) -> ReleaseCommit<'_> {
        ReleaseCommit {
            document: &mut self.document,
            mindmap_tree: &mut self.mindmap_tree,
            scene_cache: &mut self.scene_cache,
        }
    }

    /// Seed the scene cache with one entry so a `clear()` inside a
    /// commit body is visible as `cache=0` rather than being
    /// indistinguishable from "was empty anyway".
    fn seed_cache(&mut self) {
        seed_connection(
            &mut self.scene_cache,
            &EdgeKey::new("seed", "seed", "parent_child"),
        );
    }
}

/// Absolute canvas position of `node_id`'s container in the tree,
/// rounded to one decimal. `None` when there is no tree or the node
/// is not in it.
fn tree_pos(tree: &Option<MindMapTree>, node_id: &str) -> Option<(f32, f32)> {
    let tree = tree.as_ref()?;
    let arena_id = tree.arena_id_for(node_id)?;
    let pos = tree.tree.arena.get(arena_id)?.get().position();
    Some((round1(pos.x), round1(pos.y)))
}

fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

/// Render a `Vec2` at one decimal so float noise from the canvas
/// projections cannot make a trace line unstable.
fn v2(v: Vec2) -> String {
    format!("({},{})", round1(v.x), round1(v.y))
}

fn opt_v2(v: Option<Vec2>) -> String {
    match v {
        Some(v) => v2(v),
        None => "-".to_string(),
    }
}

/// One trace line. `refresh` is `None` on every step but the
/// release.
fn line(refresh: Option<ReleaseRefresh>, pending: &str, model: &str, world: &World) -> String {
    let refresh = match refresh {
        Some(r) => format!("{:?}", r),
        None => "-".to_string(),
    };
    let undo = world.document.as_ref().map(|d| d.undo_stack.len()).unwrap_or(0);
    format!(
        "{} {} model={} undo={} cache={}",
        refresh,
        pending,
        model,
        undo,
        world.scene_cache.len()
    )
}

/// One entry so a `clear()` inside a commit body is observable as
/// `cache=0` rather than being indistinguishable from "was empty
/// anyway".
///
/// One sample rather than none: an entry with no points is not
/// something the cache will store, since a refill that fills nothing
/// evicts instead.
fn seed_connection(cache: &mut SceneConnectionCache, key: &EdgeKey) {
    cache.refill(
        key,
        SampleParams {
            font_size_pt: 12.0,
            spacing: 0.0,
            sample_budget: 1000,
        },
        Vec2::ZERO,
        Vec2::ZERO,
        |out| out.push(Vec2::ZERO),
    );
}

/// Drive `script` against one gesture, rendering one trace line per
/// step.
///
/// `Move` and `Release` go through the trait — one call each, the
/// same two the event files make. The remaining hooks are what the
/// harness cannot get from the trait: a renderer-free stand-in for
/// the drain, and how this gesture's pending state and model value
/// render.
fn drive<I: ThrottledDragInteraction>(
    world: &mut World,
    interaction: &mut I,
    script: &[Step],
    drain: impl Fn(&mut I, &mut World),
    pending: impl Fn(&I) -> String,
    model: impl Fn(&World) -> String,
) -> Vec<String> {
    let mut trace = Vec::new();
    for step in script {
        let refresh = match *step {
            Step::Move { delta, cursor } => {
                interaction.accumulate(DragInput { delta, cursor });
                None
            }
            Step::Drain => {
                drain(interaction, world);
                None
            }
            Step::Release => Some(interaction.commit_on_release_core(world.commit())),
        };
        let pending = pending(interaction);
        let model = model(world);
        trace.push(line(refresh, &pending, &model, world));
    }
    trace
}

// ── Fixtures ────────────────────────────────────────────────

const FROM_ID: &str = "node-a";
const TO_ID: &str = "node-b";

/// Two nodes 400 canvas units apart joined by one `parent_child`
/// edge, so `node-b` is a real descendant of `node-a` and the
/// move-node gesture's `individual` flag has something to differ
/// about. `portal` flips the edge into portal mode with a seeded
/// `portal_from`, which is what the portal-label gesture drags.
fn edge_world(portal: bool) -> World {
    use crate::application::document::defaults::{default_orphan_node, default_parent_child_edge};

    let mut edge = default_parent_child_edge(FROM_ID, TO_ID);
    if portal {
        edge.display_mode = Some(baumhard::mindmap::model::DISPLAY_MODE_PORTAL.to_string());
        edge.portal_from = Some(baumhard::mindmap::model::PortalEndpointState::default());
    }
    let mut child = default_orphan_node(TO_ID, Vec2::new(400.0, 0.0));
    child.parent_id = Some(FROM_ID.to_string());
    let json = serde_json::json!({
        "version": "1.0",
        "name": "lifecycle-oracle",
        "canvas": {"background_color": "#000000"},
        "nodes": {
            FROM_ID: default_orphan_node(FROM_ID, Vec2::new(0.0, 0.0)),
            TO_ID: child,
        },
        "edges": [edge],
    })
    .to_string();
    let doc = MindMapDocument::from_json_str(&json, None).expect("oracle fixture JSON must parse");
    world_around(doc)
}

/// Wrap `doc` in a [`World`] with a freshly built tree and a
/// one-entry scene cache.
fn world_around(doc: MindMapDocument) -> World {
    let tree = doc.build_tree();
    let mut world = World {
        document: Some(doc),
        mindmap_tree: Some(tree),
        scene_cache: SceneConnectionCache::new(),
    };
    world.seed_cache();
    world
}

fn edge_ref() -> EdgeRef {
    EdgeRef::new(FROM_ID, TO_ID, "parent_child")
}

/// The one edge in an [`edge_world`].
fn only_edge(world: &World) -> Option<&baumhard::mindmap::model::MindEdge> {
    world.document.as_ref()?.mindmap.edges.first()
}

// ── MovingNode ──────────────────────────────────────────────

mod moving_node {
    use super::*;
    use crate::application::app::throttled_interaction::MovingNodeInteraction;
    use std::collections::HashSet;

    /// `tree0` is reported as an offset from the tree's own build
    /// position rather than an absolute: the absolute depends on
    /// font metrics and would make the trace a font regression test
    /// instead of a lifecycle one.
    fn base(world: &World) -> (f32, f32) {
        tree_pos(&world.mindmap_tree, FROM_ID).expect("fixture tree carries the node")
    }

    fn model(world: &World, base: (f32, f32)) -> String {
        let pos = |id: &str| {
            world
                .document
                .as_ref()
                .and_then(|d| d.mindmap.nodes.get(id))
                .map(|n| {
                    format!(
                        "({},{})",
                        round1(n.position.x as f32),
                        round1(n.position.y as f32)
                    )
                })
                .unwrap_or_else(|| "-".to_string())
        };
        let tree = match tree_pos(&world.mindmap_tree, FROM_ID) {
            Some((x, y)) => format!("({},{})", round1(x - base.0), round1(y - base.1)),
            None => "-".to_string(),
        };
        format!("a={} b={} tree_a+={}", pos(FROM_ID), pos(TO_ID), tree)
    }

    fn run(script: &[Step]) -> Vec<String> {
        let mut world = edge_world(false);
        let base = base(&world);
        let mut i = MovingNodeInteraction::new(
            vec![FROM_ID.to_string()],
            false,
            HashSet::from([TO_ID.to_string()]),
        );
        drive(
            &mut world,
            &mut i,
            script,
            |i, w| {
                if let Some(tree) = w.mindmap_tree.as_mut() {
                    for nid in &i.node_ids {
                        let d = i.pending.pending_delta();
                        apply_drag_delta(tree, nid, d.x, d.y, !i.individual);
                    }
                }
                i.pending.take_delta();
            },
            |i| {
                format!(
                    "pend={} total={}",
                    v2(i.pending.pending_delta()),
                    v2(i.pending.total_delta())
                )
            },
            |w| model(w, base),
        )
    }

    /// No drain landed: the release owns the whole flush, so the
    /// tree gains the full delta and the cache clears.
    #[test]
    fn test_release_without_any_drain_flushes_everything() {
        assert_eq!(
            run(&[mv(10.0, 5.0, 10.0, 5.0), mv(2.0, 3.0, 12.0, 8.0), Step::Release]),
            vec![
                "- pend=(10,5) total=(10,5) model=a=(0,0) b=(400,0) tree_a+=(0,0) undo=0 cache=1",
                "- pend=(12,8) total=(12,8) model=a=(0,0) b=(400,0) tree_a+=(0,0) undo=0 cache=1",
                "All pend=(0,0) total=(12,8) model=a=(12,8) b=(412,8) tree_a+=(12,8) undo=1 cache=0",
            ]
        );
    }

    /// A drain took the first sample; the release flushes only what
    /// arrived after it, and still clears the cache because
    /// something was pending.
    #[test]
    fn test_release_after_partial_drain_flushes_the_remainder() {
        assert_eq!(
            run(&[
                mv(10.0, 5.0, 10.0, 5.0),
                Step::Drain,
                mv(2.0, 3.0, 12.0, 8.0),
                Step::Release
            ]),
            vec![
                "- pend=(10,5) total=(10,5) model=a=(0,0) b=(400,0) tree_a+=(0,0) undo=0 cache=1",
                "- pend=(0,0) total=(10,5) model=a=(0,0) b=(400,0) tree_a+=(10,5) undo=0 cache=1",
                "- pend=(2,3) total=(12,8) model=a=(0,0) b=(400,0) tree_a+=(10,5) undo=0 cache=1",
                "All pend=(0,0) total=(12,8) model=a=(12,8) b=(412,8) tree_a+=(12,8) undo=1 cache=0",
            ]
        );
    }

    /// The drain caught the last sample: nothing pending at release,
    /// so the tree is already current and the cache is **kept**.
    /// The conditional clear is the whole point of the `had_pending`
    /// flag — an unconditional clear here would be a silently
    /// different (and more expensive) release.
    #[test]
    fn test_release_with_nothing_pending_keeps_the_scene_cache() {
        assert_eq!(
            run(&[mv(10.0, 5.0, 10.0, 5.0), Step::Drain, Step::Release]),
            vec![
                "- pend=(10,5) total=(10,5) model=a=(0,0) b=(400,0) tree_a+=(0,0) undo=0 cache=1",
                "- pend=(0,0) total=(10,5) model=a=(0,0) b=(400,0) tree_a+=(10,5) undo=0 cache=1",
                "All pend=(0,0) total=(10,5) model=a=(10,5) b=(410,5) tree_a+=(10,5) undo=1 cache=1",
            ]
        );
    }

    /// Alt-drag: only the anchor moves, in the model and in the
    /// tree.
    #[test]
    fn test_individual_drag_leaves_the_descendant_where_it_was() {
        let mut world = edge_world(false);
        let base = base(&world);
        let mut i = MovingNodeInteraction::new(vec![FROM_ID.to_string()], true, HashSet::new());
        assert_eq!(
            drive(
                &mut world,
                &mut i,
                &[mv(10.0, 5.0, 10.0, 5.0), Step::Release],
                |i, _w| {
                    i.pending.take_delta();
                },
                |i| {
                    format!(
                        "pend={} total={}",
                        v2(i.pending.pending_delta()),
                        v2(i.pending.total_delta())
                    )
                },
                |w| model(w, base),
            ),
            vec![
                "- pend=(10,5) total=(10,5) model=a=(0,0) b=(400,0) tree_a+=(0,0) undo=0 cache=1",
                "All pend=(0,0) total=(10,5) model=a=(10,5) b=(400,0) tree_a+=(10,5) undo=1 cache=0",
            ]
        );
    }

    /// No document loaded: the tree flush still runs (it is ordered
    /// before the document check), nothing commits, and the decree
    /// is `None`.
    #[test]
    fn test_release_without_a_document_flushes_the_tree_and_commits_nothing() {
        let mut world = edge_world(false);
        let base = base(&world);
        world.document = None;
        let mut i = MovingNodeInteraction::new(vec![FROM_ID.to_string()], false, HashSet::new());
        assert_eq!(
            drive(
                &mut world,
                &mut i,
                &[mv(10.0, 5.0, 10.0, 5.0), Step::Release],
                |i, _w| {
                    i.pending.take_delta();
                },
                |i| {
                    format!(
                        "pend={} total={}",
                        v2(i.pending.pending_delta()),
                        v2(i.pending.total_delta())
                    )
                },
                |w| model(w, base),
            ),
            vec![
                "- pend=(10,5) total=(10,5) model=a=- b=- tree_a+=(0,0) undo=0 cache=1",
                "None pend=(0,0) total=(10,5) model=a=- b=- tree_a+=(10,5) undo=0 cache=1",
            ]
        );
    }
}

// ── MovingSection / SectionResize / NodeResize ──────────────
//
// The three tree-only gestures. Their drains write the tree and
// nothing the release reads back — the release recomputes from
// `start_*` plus `total_delta` — so a `Drain` step here models the
// pending clear alone. What the traces pin is that the release
// commit is a pure function of `total_delta` regardless of how the
// throttle sliced the samples, and that the cache clear is
// unconditional (unlike `MovingNode`'s).

mod section_gestures {
    use super::*;
    use crate::application::app::throttled_interaction::{
        MovingSectionInteraction, SectionResizeInteraction,
    };
    use crate::application::document::tests_common::pinned_two_section_node;
    use baumhard::mindmap::model::{Position, Size};

    const SECTION_IDX: usize = 1;

    /// The two-section fixture: the node is 200x100, section 1 sits
    /// at offset (10,10) sized 50x30. Every number in the traces
    /// below derives from those.
    fn section_world() -> (World, String) {
        let (doc, id) = pinned_two_section_node();
        (world_around(doc), id)
    }

    fn section_model(world: &World, id: &str) -> String {
        let Some(section) = world
            .document
            .as_ref()
            .and_then(|d| d.mindmap.nodes.get(id))
            .and_then(|n| n.sections.get(SECTION_IDX))
        else {
            return "-".to_string();
        };
        let size = match &section.size {
            Some(s) => format!("({},{})", round1(s.width as f32), round1(s.height as f32)),
            None => "fill".to_string(),
        };
        format!(
            "off=({},{}) size={}",
            round1(section.offset.x as f32),
            round1(section.offset.y as f32),
            size
        )
    }

    // ── MovingSection ───────────────────────────────────────

    fn run_move(script: &[Step]) -> Vec<String> {
        let (mut world, id) = section_world();
        let mut i = MovingSectionInteraction::new(id.clone(), SECTION_IDX, (10.0, 10.0));
        let trace_id = id.clone();
        drive(
            &mut world,
            &mut i,
            script,
            |i, _w| {
                i.pending.take_delta();
            },
            |i| {
                format!(
                    "pend={} total={}",
                    v2(i.pending.pending_delta()),
                    v2(i.pending.total_delta())
                )
            },
            move |w| section_model(w, &trace_id),
        )
    }

    /// `start_offset` plus `total_delta`, one setter call, one undo
    /// entry, cache cleared unconditionally.
    #[test]
    fn test_moving_section_commits_start_offset_plus_total() {
        assert_eq!(
            run_move(&[mv(5.0, 4.0, 5.0, 4.0), mv(3.0, 1.0, 8.0, 5.0), Step::Release]),
            vec![
                "- pend=(5,4) total=(5,4) model=off=(10,10) size=(50,30) undo=0 cache=1",
                "- pend=(8,5) total=(8,5) model=off=(10,10) size=(50,30) undo=0 cache=1",
                "All pend=(8,5) total=(8,5) model=off=(18,15) size=(50,30) undo=1 cache=0",
            ]
        );
    }

    /// A drain in the middle changes nothing about the commit — the
    /// release does not read pending at all. Same final offset, same
    /// undo depth, same cleared cache as the drain-free run.
    #[test]
    fn test_moving_section_release_ignores_how_the_throttle_sliced_it() {
        assert_eq!(
            run_move(&[
                mv(5.0, 4.0, 5.0, 4.0),
                Step::Drain,
                mv(3.0, 1.0, 8.0, 5.0),
                Step::Release
            ]),
            vec![
                "- pend=(5,4) total=(5,4) model=off=(10,10) size=(50,30) undo=0 cache=1",
                "- pend=(0,0) total=(5,4) model=off=(10,10) size=(50,30) undo=0 cache=1",
                "- pend=(3,1) total=(8,5) model=off=(10,10) size=(50,30) undo=0 cache=1",
                "All pend=(3,1) total=(8,5) model=off=(18,15) size=(50,30) undo=1 cache=0",
            ]
        );
    }

    /// A zero-delta release is a committed no-op: the setter reports
    /// `Ok(false)`, no undo entry is pushed, and the cache still
    /// clears.
    #[test]
    fn test_moving_section_zero_delta_release_pushes_no_undo() {
        assert_eq!(
            run_move(&[Step::Release]),
            vec!["All pend=(0,0) total=(0,0) model=off=(10,10) size=(50,30) undo=0 cache=0"]
        );
    }

    // ── SectionResize ───────────────────────────────────────

    fn run_section_resize(side: ResizeHandleSide, script: &[Step]) -> Vec<String> {
        let (mut world, id) = section_world();
        let mut i = SectionResizeInteraction::new(
            id.clone(),
            SECTION_IDX,
            side,
            Position { x: 10.0, y: 10.0 },
            Size {
                width: 50.0,
                height: 30.0,
            },
            false,
        );
        let trace_id = id.clone();
        drive(
            &mut world,
            &mut i,
            script,
            |i, _w| {
                i.pending.take_delta();
            },
            |i| {
                format!(
                    "pend={} total={}",
                    v2(i.pending.pending_delta()),
                    v2(i.pending.total_delta())
                )
            },
            move |w| section_model(w, &trace_id),
        )
    }

    /// SE grows the size and leaves the offset alone.
    #[test]
    fn test_section_resize_se_grows_size_only() {
        assert_eq!(
            run_section_resize(ResizeHandleSide::SE, &[mv(20.0, 10.0, 20.0, 10.0), Step::Release]),
            vec![
                "- pend=(20,10) total=(20,10) model=off=(10,10) size=(50,30) undo=0 cache=1",
                "All pend=(20,10) total=(20,10) model=off=(10,10) size=(70,40) undo=1 cache=0",
            ]
        );
    }

    /// NW shifts the offset and shrinks the size in one atomic
    /// `set_section_aabb` write — the two-step offset-then-size path
    /// this replaced rejected the intermediate state.
    #[test]
    fn test_section_resize_nw_moves_offset_and_size_atomically() {
        assert_eq!(
            run_section_resize(
                ResizeHandleSide::NW,
                &[mv(5.0, 4.0, 5.0, 4.0), Step::Drain, Step::Release]
            ),
            vec![
                "- pend=(5,4) total=(5,4) model=off=(10,10) size=(50,30) undo=0 cache=1",
                "- pend=(0,0) total=(5,4) model=off=(10,10) size=(50,30) undo=0 cache=1",
                "All pend=(0,0) total=(5,4) model=off=(15,14) size=(45,26) undo=1 cache=0",
            ]
        );
    }

    /// A rejected commit (dragged past the parent's right edge)
    /// leaves the model untouched, pushes no undo entry, and still
    /// clears the cache so the rebuild snaps the section back.
    #[test]
    fn test_section_resize_rejected_commit_snaps_back_without_undo() {
        assert_eq!(
            run_section_resize(ResizeHandleSide::SE, &[mv(500.0, 0.0, 500.0, 0.0), Step::Release]),
            vec![
                "- pend=(500,0) total=(500,0) model=off=(10,10) size=(50,30) undo=0 cache=1",
                "All pend=(500,0) total=(500,0) model=off=(10,10) size=(50,30) undo=0 cache=0",
            ]
        );
    }
}

mod node_resize {
    use super::*;
    use crate::application::app::throttled_interaction::NodeResizeInteraction;
    use baumhard::mindmap::model::{Position, Size};

    fn node_model(world: &World) -> String {
        let Some(node) = world.document.as_ref().and_then(|d| d.mindmap.nodes.get(FROM_ID)) else {
            return "-".to_string();
        };
        format!(
            "pos=({},{}) size=({},{})",
            round1(node.position.x as f32),
            round1(node.position.y as f32),
            round1(node.size.width as f32),
            round1(node.size.height as f32)
        )
    }

    fn run(side: ResizeHandleSide, started_with_right: bool, script: &[Step]) -> Vec<String> {
        let mut world = edge_world(false);
        let (start_position, start_size) = {
            let n = &world.document.as_ref().unwrap().mindmap.nodes[FROM_ID];
            (n.position, n.size)
        };
        let mut i = NodeResizeInteraction::new(
            FROM_ID.to_string(),
            side,
            start_position,
            start_size,
            started_with_right,
        );
        drive(
            &mut world,
            &mut i,
            script,
            |i, _w| {
                i.pending.take_delta();
            },
            |i| {
                format!(
                    "pend={} total={}",
                    v2(i.pending.pending_delta()),
                    v2(i.pending.total_delta())
                )
            },
            node_model,
        )
    }

    /// The fixture node's authored AABB, so the expectations below
    /// read as deltas from a stated starting point rather than
    /// magic numbers.
    fn start() -> (Position, Size) {
        let world = edge_world(false);
        let n = &world.document.as_ref().unwrap().mindmap.nodes[FROM_ID];
        (n.position, n.size)
    }

    #[test]
    fn test_fixture_node_starts_at_a_known_aabb() {
        let (pos, size) = start();
        assert_eq!((pos.x, pos.y), (0.0, 0.0));
        assert_eq!((size.width, size.height), (240.0, 60.0));
    }

    #[test]
    fn test_node_resize_se_grows_size_only() {
        assert_eq!(
            run(
                ResizeHandleSide::SE,
                false,
                &[mv(20.0, 10.0, 20.0, 10.0), Step::Release]
            ),
            vec![
                "- pend=(20,10) total=(20,10) model=pos=(0,0) size=(240,60) undo=0 cache=1",
                "All pend=(20,10) total=(20,10) model=pos=(0,0) size=(260,70) undo=1 cache=0",
            ]
        );
    }

    /// A right-button fast-resize commits through exactly the same
    /// body as a left-button handle drag — the only thing
    /// `started_with_right` changes is the log label.
    #[test]
    fn test_fast_resize_commits_identically_to_the_handle_drag() {
        let left = run(
            ResizeHandleSide::SE,
            false,
            &[mv(20.0, 10.0, 20.0, 10.0), Step::Release],
        );
        let right = run(
            ResizeHandleSide::SE,
            true,
            &[mv(20.0, 10.0, 20.0, 10.0), Step::Release],
        );
        assert_eq!(left, right);
    }

    /// NW shifts position and shrinks size.
    #[test]
    fn test_node_resize_nw_shifts_position_and_shrinks() {
        assert_eq!(
            run(
                ResizeHandleSide::NW,
                false,
                &[mv(5.0, 4.0, 5.0, 4.0), Step::Drain, Step::Release]
            ),
            vec![
                "- pend=(5,4) total=(5,4) model=pos=(0,0) size=(240,60) undo=0 cache=1",
                "- pend=(0,0) total=(5,4) model=pos=(0,0) size=(240,60) undo=0 cache=1",
                "All pend=(0,0) total=(5,4) model=pos=(5,4) size=(235,56) undo=1 cache=0",
            ]
        );
    }

    /// Dragging a corner past the opposite edge produces a
    /// non-positive size; the setter rejects it, nothing commits,
    /// and the cache still clears so the rebuild snaps back.
    #[test]
    fn test_node_resize_rejected_commit_snaps_back_without_undo() {
        assert_eq!(
            run(
                ResizeHandleSide::SE,
                false,
                &[mv(-500.0, -500.0, -500.0, -500.0), Step::Release]
            ),
            vec![
                "- pend=(-500,-500) total=(-500,-500) model=pos=(0,0) size=(240,60) undo=0 cache=1",
                "All pend=(-500,-500) total=(-500,-500) model=pos=(0,0) size=(240,60) undo=0 cache=0",
            ]
        );
    }
}

// ── EdgeHandle ──────────────────────────────────────────────

mod edge_handle {
    use super::*;
    use crate::application::app::edge_drag::apply_edge_handle_drag;
    use crate::application::app::throttled_interaction::EdgeHandleInteraction;
    use baumhard::mindmap::tree_builder::EdgeHandleKind;

    fn cp_model(world: &World) -> String {
        let Some(edge) = only_edge(world) else {
            return "-".to_string();
        };
        let cps: Vec<String> = edge
            .control_points
            .iter()
            .map(|c| format!("({},{})", round1(c.x as f32), round1(c.y as f32)))
            .collect();
        format!("cps=[{}]", cps.join(" "))
    }

    fn run(handle: EdgeHandleKind, script: &[Step]) -> Vec<String> {
        let mut world = edge_world(false);
        let mut i = EdgeHandleInteraction::new(edge_ref(), handle, seed_edge(&world), Vec2::ZERO);
        drive(
            &mut world,
            &mut i,
            script,
            |i, w| {
                if let Some(doc) = w.document.as_mut() {
                    i.handle = apply_edge_handle_drag(
                        doc,
                        &i.edge_ref,
                        i.handle,
                        i.start_handle_pos,
                        i.pending.total_delta(),
                    );
                }
                i.pending.take_delta();
            },
            |i| {
                format!(
                    "pend={} total={}",
                    v2(i.pending.pending_delta()),
                    v2(i.pending.total_delta())
                )
            },
            cp_model,
        )
    }

    fn seed_edge(world: &World) -> baumhard::mindmap::model::MindEdge {
        only_edge(world).expect("fixture carries one edge").clone()
    }

    /// Dragging a fresh `Midpoint` inserts one control point,
    /// offset from the source node's center, and commits
    /// unconditionally — crossing the drag threshold is taken as
    /// proof of a state change.
    #[test]
    fn test_midpoint_drag_inserts_a_control_point_and_always_commits() {
        assert_eq!(
            run(
                EdgeHandleKind::Midpoint,
                &[mv(30.0, 20.0, 30.0, 20.0), Step::Release]
            ),
            vec![
                "- pend=(30,20) total=(30,20) model=cps=[] undo=0 cache=1",
                "All pend=(30,20) total=(30,20) model=cps=[(-90,-10)] undo=1 cache=1",
            ]
        );
    }

    /// The release re-applies from `total_delta`, so a drain that
    /// already wrote the same offset is idempotent: the model lands
    /// in the same place and exactly one undo entry is pushed.
    #[test]
    fn test_release_reapplies_total_delta_idempotently_after_a_drain() {
        assert_eq!(
            run(
                EdgeHandleKind::Midpoint,
                &[
                    mv(30.0, 20.0, 30.0, 20.0),
                    Step::Drain,
                    mv(10.0, 0.0, 40.0, 20.0),
                    Step::Release
                ]
            ),
            vec![
                "- pend=(30,20) total=(30,20) model=cps=[] undo=0 cache=1",
                "- pend=(0,0) total=(30,20) model=cps=[(-90,-10)] undo=0 cache=1",
                "- pend=(10,0) total=(40,20) model=cps=[(-90,-10)] undo=0 cache=1",
                "All pend=(10,0) total=(40,20) model=cps=[(-80,-10)] undo=1 cache=1",
            ]
        );
    }

    /// The edge-handle release is the one commit that does **not**
    /// clear the scene cache: its own drain invalidates the single
    /// dirty edge key by hand, so a whole-cache flush would throw
    /// away every other edge's samples for nothing.
    #[test]
    fn test_release_leaves_the_scene_cache_alone() {
        let trace = run(
            EdgeHandleKind::ControlPoint(0),
            &[mv(5.0, 5.0, 5.0, 5.0), Step::Release],
        );
        assert!(
            trace.last().unwrap().ends_with("cache=1"),
            "edge-handle release must not clear the cache: {:?}",
            trace
        );
    }
}

// ── PortalLabel ─────────────────────────────────────────────

mod portal_label {
    use super::*;
    use crate::application::app::portal_label_drag::apply_portal_label_drag;
    use crate::application::app::throttled_interaction::PortalLabelInteraction;

    fn portal_model(world: &World) -> String {
        let Some(edge) = only_edge(world) else {
            return "-".to_string();
        };
        let render = |s: &Option<baumhard::mindmap::model::PortalEndpointState>| match s {
            Some(s) => format!(
                "t={} perp={}",
                s.border_t
                    .map(round1)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into()),
                s.perpendicular_offset
                    .map(round1)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into())
            ),
            None => "-".to_string(),
        };
        format!(
            "from[{}] to[{}]",
            render(&edge.portal_from),
            render(&edge.portal_to)
        )
    }

    fn run(script: &[Step]) -> Vec<String> {
        let mut world = edge_world(true);
        let original = only_edge(&world).expect("fixture carries one edge").clone();
        let mut i = PortalLabelInteraction::new(edge_ref(), FROM_ID.to_string(), original);
        drive(
            &mut world,
            &mut i,
            script,
            |i, w| {
                if let (Some(doc), Some(cursor)) = (w.document.as_mut(), i.pending.take_cursor()) {
                    apply_portal_label_drag(doc, &i.edge_ref, &i.endpoint_node_id, cursor);
                }
            },
            |i| format!("cursor={}", opt_v2(i.pending.peek_cursor())),
            portal_model,
        )
    }

    /// Overwrite discipline: the latch holds the last cursor, not a
    /// queue, and the release commits from that one.
    #[test]
    fn test_latched_cursor_is_the_last_one_and_the_release_commits_it() {
        assert_eq!(
            run(&[mv(0.0, 0.0, 60.0, 30.0), mv(0.0, 0.0, 120.0, 30.0), Step::Release]),
            vec![
                "- cursor=(60,30) model=from[t=- perp=-] to[-] undo=0 cache=1",
                "- cursor=(120,30) model=from[t=- perp=-] to[-] undo=0 cache=1",
                "All cursor=- model=from[t=0.5 perp=-30] to[-] undo=1 cache=1",
            ]
        );
    }

    /// The drain consumed the only cursor, so the release has
    /// nothing to flush — but it still runs the commit, and the
    /// no-op predicate (which compares `portal_from` / `portal_to`
    /// only) sees the drain's write and pushes the undo entry.
    #[test]
    fn test_release_after_a_drain_still_commits_the_drained_write() {
        assert_eq!(
            run(&[mv(0.0, 0.0, 60.0, 30.0), Step::Drain, Step::Release]),
            vec![
                "- cursor=(60,30) model=from[t=- perp=-] to[-] undo=0 cache=1",
                "- cursor=- model=from[t=0.3 perp=-30] to[-] undo=0 cache=1",
                "All cursor=- model=from[t=0.3 perp=-30] to[-] undo=1 cache=1",
            ]
        );
    }

    /// A release with nothing latched and nothing drained finds the
    /// edge unchanged, so the predicate suppresses the undo entry.
    #[test]
    fn test_release_with_nothing_to_commit_pushes_no_undo() {
        assert_eq!(
            run(&[Step::Release]),
            vec!["All cursor=- model=from[t=- perp=-] to[-] undo=0 cache=1"]
        );
    }
}

// ── EdgeLabel ───────────────────────────────────────────────

mod edge_label {
    use super::*;
    use crate::application::app::edge_label_drag::apply_edge_label_drag;
    use crate::application::app::throttled_interaction::EdgeLabelInteraction;

    fn label_model(world: &World) -> String {
        let Some(edge) = only_edge(world) else {
            return "-".to_string();
        };
        match &edge.label_config {
            Some(cfg) => format!(
                "t={} perp={}",
                cfg.position_t
                    .map(round1)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into()),
                cfg.perpendicular_offset
                    .map(round1)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into())
            ),
            None => "-".to_string(),
        }
    }

    fn run(script: &[Step]) -> Vec<String> {
        let mut world = edge_world(false);
        let original = only_edge(&world).expect("fixture carries one edge").clone();
        let mut i = EdgeLabelInteraction::new(edge_ref(), original);
        drive(
            &mut world,
            &mut i,
            script,
            |i, w| {
                if let (Some(doc), Some(cursor)) = (w.document.as_mut(), i.pending.take_cursor()) {
                    apply_edge_label_drag(doc, &i.edge_ref, cursor);
                }
            },
            |i| format!("cursor={}", opt_v2(i.pending.peek_cursor())),
            label_model,
        )
    }

    /// The edge-label release is the only one that asks for a
    /// scene-only rebuild — node trees are untouched by a label
    /// move. It also leaves the scene cache alone.
    #[test]
    fn test_release_commits_the_latched_cursor_and_asks_for_scene_only() {
        assert_eq!(
            run(&[mv(0.0, 0.0, 300.0, 40.0), Step::Release]),
            vec![
                "- cursor=(300,40) model=- undo=0 cache=1",
                "SceneOnly cursor=- model=t=0.4 perp=10 undo=1 cache=1",
            ]
        );
    }

    /// Drain-then-release: the drain took the cursor and wrote the
    /// config; the release finds the same change and pushes exactly
    /// one undo entry.
    #[test]
    fn test_release_after_a_drain_commits_once() {
        assert_eq!(
            run(&[mv(0.0, 0.0, 300.0, 40.0), Step::Drain, Step::Release]),
            vec![
                "- cursor=(300,40) model=- undo=0 cache=1",
                "- cursor=- model=t=0.4 perp=10 undo=0 cache=1",
                "SceneOnly cursor=- model=t=0.4 perp=10 undo=1 cache=1",
            ]
        );
    }

    /// Nothing latched, nothing drained: the `label_config`
    /// comparison finds no change and the undo entry is skipped.
    #[test]
    fn test_release_with_nothing_to_commit_pushes_no_undo() {
        assert_eq!(
            run(&[Step::Release]),
            vec!["SceneOnly cursor=- model=- undo=0 cache=1"]
        );
    }
}

// ── The release gate ────────────────────────────────────────

/// The button gate wrapped around the seven commits, driven the way
/// `event_mouse_click::finalize_or_put_back` drives it.
///
/// The full shell takes `&mut InputHandlerContext` — a live wgpu
/// device — so it stays out of reach (TEST_CONVENTIONS §T8).
/// [`commit_if_resolved`] is its renderer-free half, and it is the
/// half that decides whether anything is written at all. Pinned here
/// because a gate that committed on *both* branches would end a
/// left-button drag on a stray right-click and write its model
/// besides, and no per-interaction test can see that: every commit
/// body is perfectly correct in isolation. The bug lives entirely in
/// which of them runs.
///
/// The gate owes two things, and both are pinned here: *whether* a
/// commit runs (the four button × origin cells) and *what it hands
/// back* once one has. The second needs a gesture whose decree is
/// not the majority answer, or a gate that ignored its core and
/// returned a constant `Some(All)` would pass every cell — hence the
/// `EdgeLabel` row at the end.
mod release_gate {
    use super::*;
    use crate::application::app::throttled_interaction::release::commit_if_resolved;
    use crate::application::app::throttled_interaction::{
        EdgeLabelInteraction, MovingNodeInteraction, NodeResizeInteraction, ThrottledDrag,
    };
    use crate::application::platform::input::MouseButton;
    use baumhard::mindmap::model::{Position, Size};
    use baumhard::mindmap::tree_builder::ResizeHandleSide;
    use std::collections::HashSet;

    /// A left-started move-node carrying 30 units the throttle never
    /// flushed. Committing it is loud — the node moves and an undo
    /// entry appears — so "committed anyway" cannot hide.
    fn left_started_drag() -> ThrottledDrag {
        let mut i = MovingNodeInteraction::new(
            vec![FROM_ID.to_string()],
            false,
            HashSet::from([TO_ID.to_string()]),
        );
        i.accumulate(DragInput {
            delta: Vec2::new(30.0, 0.0),
            cursor: Vec2::new(30.0, 0.0),
        });
        ThrottledDrag::MovingNode(i)
    }

    /// A left-started edge-label drag with a latched cursor — the
    /// one gesture of the seven whose commit answers anything other
    /// than [`ReleaseRefresh::All`]. Takes the world because the
    /// interaction is constructed from the edge it will rewrite.
    fn edge_label_drag(world: &World) -> ThrottledDrag {
        let original = only_edge(world).expect("fixture carries one edge").clone();
        let mut i = EdgeLabelInteraction::new(edge_ref(), original);
        i.accumulate(DragInput {
            delta: Vec2::ZERO,
            cursor: Vec2::new(200.0, 30.0),
        });
        ThrottledDrag::EdgeLabel(i)
    }

    /// A right-started fast-resize: the one shape the right button
    /// *does* own.
    fn right_started_drag() -> ThrottledDrag {
        let mut i = NodeResizeInteraction::new(
            FROM_ID.to_string(),
            ResizeHandleSide::SE,
            Position { x: 0.0, y: 0.0 },
            Size {
                width: 100.0,
                height: 50.0,
            },
            true,
        );
        i.accumulate(DragInput {
            delta: Vec2::new(20.0, 10.0),
            cursor: Vec2::new(20.0, 10.0),
        });
        ThrottledDrag::NodeResize(i)
    }

    fn node_pos(world: &World) -> (f64, f64) {
        let n = world
            .document
            .as_ref()
            .and_then(|d| d.mindmap.nodes.get(FROM_ID))
            .expect("fixture carries the node");
        (n.position.x, n.position.y)
    }

    fn node_size(world: &World) -> (f64, f64) {
        let n = world
            .document
            .as_ref()
            .and_then(|d| d.mindmap.nodes.get(FROM_ID))
            .expect("fixture carries the node");
        (n.size.width, n.size.height)
    }

    fn undo_depth(world: &World) -> usize {
        world.document.as_ref().map(|d| d.undo_stack.len()).unwrap_or(0)
    }

    /// **The put-back branch writes nothing.** A right release
    /// arriving part-way through a left-button drag must hand the
    /// gesture back untouched: no model write, no undo entry, no
    /// scene-cache clear, and no decree for the caller to run.
    ///
    /// This is the one the shell could get wrong without any
    /// interaction noticing — `commit_on_release_core` would do
    /// exactly what it is supposed to, on a release that had no
    /// business calling it.
    #[test]
    fn test_right_release_on_a_left_started_drag_commits_nothing() {
        let mut world = edge_world(false);
        let before_pos = node_pos(&world);
        let mut drag = left_started_drag();

        let refresh = commit_if_resolved(MouseButton::Right, &mut drag, world.commit());

        assert_eq!(refresh, None, "a put-back owes the canvas no decree");
        assert_eq!(undo_depth(&world), 0, "a put-back must push no undo entry");
        assert_eq!(node_pos(&world), before_pos, "a put-back must not move the node");
        assert_eq!(
            world.scene_cache.len(),
            1,
            "a put-back must leave the scene cache alone"
        );
    }

    /// The converse, on the same gesture and the same world: the
    /// left release does commit, so the assertions above are pinning
    /// the gate and not an inert fixture.
    #[test]
    fn test_left_release_on_the_same_drag_commits() {
        let mut world = edge_world(false);
        let before_pos = node_pos(&world);
        let mut drag = left_started_drag();

        let refresh = commit_if_resolved(MouseButton::Left, &mut drag, world.commit());

        assert_eq!(refresh, Some(ReleaseRefresh::All));
        assert_eq!(undo_depth(&world), 1, "the commit pushes its undo entry");
        assert_eq!(
            node_pos(&world),
            (before_pos.0 + 30.0, before_pos.1),
            "the commit applies the full accumulated delta"
        );
        assert_eq!(
            world.scene_cache.len(),
            0,
            "the commit clears the cache it left stale samples in"
        );
    }

    /// A right release *does* own the fast-resize it started, so the
    /// gate is not simply "the right button never commits".
    ///
    /// Only the height is asserted exactly: `set_node_aabb` clamps
    /// the width up to the node's text-driven minimum, which is a
    /// font metric and would make this a font regression test.
    #[test]
    fn test_right_release_commits_the_fast_resize_it_started() {
        let mut world = edge_world(false);
        let mut drag = right_started_drag();

        let refresh = commit_if_resolved(MouseButton::Right, &mut drag, world.commit());

        assert_eq!(refresh, Some(ReleaseRefresh::All));
        assert_eq!(undo_depth(&world), 1);
        assert_eq!(
            node_size(&world).1,
            60.0,
            "the SE handle's 10 units of vertical drag reached the model"
        );
    }

    /// ...and the same fast-resize is *not* finalized by a left
    /// release, which is the fourth cell of the gate and the one
    /// #37 turned over.
    ///
    /// The cell became reachable when the left press started being
    /// refused mid-gesture (`DragState::would_abandon_gesture`):
    /// refusing the press is what lets the right-started drag
    /// survive to see a left release at all. Committing there ends a
    /// resize the user is still performing with the right button
    /// down, and writes the model at whatever the pointer had
    /// reached — the same silent loss the right-release cell above
    /// rejects, mirrored.
    #[test]
    fn test_left_release_hands_back_a_right_started_drag() {
        let mut world = edge_world(false);
        let before_size = node_size(&world);
        let mut drag = right_started_drag();

        let refresh = commit_if_resolved(MouseButton::Left, &mut drag, world.commit());

        assert_eq!(refresh, None, "a put-back owes the canvas no decree");
        assert_eq!(undo_depth(&world), 0, "a put-back must push no undo entry");
        assert_eq!(
            node_size(&world),
            before_size,
            "a put-back must not resize the node"
        );
    }

    /// **The gate carries the decree it is handed, it does not
    /// invent one.** The four cases above pin *whether* a commit
    /// runs, and both of the gestures they drive answer
    /// [`ReleaseRefresh::All`] — so a gate that ignored its core and
    /// returned a constant `Some(All)` would be indistinguishable
    /// from the shipped one in every other test in this crate.
    ///
    /// `EdgeLabel` is the only gesture of the seven that asks for
    /// anything else, and the disagreement is the point: an
    /// edge-label move touches nothing but the scene projection, so
    /// committing it must rebuild the scene and not walk the node
    /// trees. Running it through the gate is what pins the carry.
    #[test]
    fn test_the_gate_carries_the_scene_only_decree_of_an_edge_label_release() {
        let mut world = edge_world(false);
        let mut drag = edge_label_drag(&world);

        let refresh = commit_if_resolved(MouseButton::Left, &mut drag, world.commit());

        assert_eq!(
            refresh,
            Some(ReleaseRefresh::SceneOnly),
            "the gate must hand back the core's own decree, not a constant"
        );
        assert_eq!(undo_depth(&world), 1, "the label move is still committed");
    }
}
