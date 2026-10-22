// SPDX-License-Identifier: MPL-2.0

//! Pointer-gesture apply_* helpers — the bodies mouse and touch
//! gestures run once their `MouseGesture` name has resolved to an
//! `Action` through the keybind table.
//!
//! [`DispatchHit`] lives here rather than in the native dispatcher
//! because both targets now populate one: the payload is what a
//! keyboard-driven dispatch cannot carry (what the press landed on,
//! and where it landed in canvas space), and it is made of plain
//! values that exist on both targets.
//!
//! [`resolve_double_click_route`] is the pure half of the
//! double-click behavior — the decision, with no renderer in sight,
//! so `cargo test` can pin every branch (`TEST_CONVENTIONS §T8`).
//! [`apply_double_click_activate`] is the renderer-driving half and
//! matches exhaustively on the route, so a new [`DoubleClickRoute`]
//! variant is a build error rather than a silently-dropped gesture.

use glam::Vec2;

use crate::application::common::RenderDecree;
use crate::application::document::{
    hit_test_edge, EdgeLabelSel, EdgeRef, PortalLabelSel, SectionSel, SelectionState,
};
use crate::application::keybinds::{Action, MouseGesture, ResolvedKeybinds};
use baumhard::mindmap::scene_cache::EdgeKey;

use crate::application::app::click_triggers::fire_onclick_triggers;
use crate::application::app::input_context_core::InputContextCore;
use crate::application::app::scene_rebuild::{rebuild_after_selection_change, rebuild_all, RebuildTier};
use crate::application::app::touch_gesture::{Phase, RecognizedGesture, TouchGestureRecognizer};
use crate::application::app::{
    compute_click_hit, now_ms, ClickHit, ClickHitParts, InteractionMode, EDGE_HIT_TOLERANCE_PX,
    PLATFORM_CONTEXT,
};

use super::apply_create_orphan_node_and_edit;
use super::canvas_per_pixel::CanvasPerPixel;

/// Per-event payload that pointer-driven Actions need but keyboard
/// dispatch doesn't. Populated by the mouse handlers on both
/// targets right before they call into the dispatcher; `None` for
/// keyboard / macro / touch callers.
#[derive(Debug, Clone)]
pub struct DispatchHit {
    /// What the click landed on. The `DoubleClickActivate` arm
    /// routes on this.
    pub click_hit: ClickHit,
    /// Canvas-space cursor position at the gesture's trigger time.
    /// Used by orphan-creation / open-editor arms.
    pub canvas_pos: Vec2,
}

/// What a double-click does, decided before anything is mutated.
///
/// Pure output of [`resolve_double_click_route`]: every variant
/// carries the identity the apply half needs, so the apply half
/// never re-inspects the `ClickHit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::application::app) enum DoubleClickRoute {
    /// Select the pointed-at node (section-aware) and open the
    /// inline node text editor on it, seeded with the existing text.
    OpenNodeEditor {
        node_id: String,
        /// `Some(idx)` when the press resolved to a specific section
        /// of a multi-section node. Preserved so the editor opens on
        /// the section the user pointed at rather than section 0.
        section_idx: Option<usize>,
    },
    /// Center the camera on the *other* endpoint of a portal-mode
    /// edge and select that edge. Icon and text sub-parts share this
    /// route — same endpoint identity, same "navigate" intent.
    PanToPortalPartner { edge: EdgeKey, partner_id: String },
    /// Select the edge label and edit it. The selection commit is
    /// cross-platform; the editor open is native-only today.
    EditEdgeLabel { edge_ref: EdgeRef },
    /// Empty canvas, no edge selected, and the user has explicitly
    /// bound `CreateOrphanNodeAndEdit` somewhere: create a node at
    /// the press position and open its editor clean.
    CreateOrphanAndEdit,
    /// The gesture is a deliberate no-op. Reached for empty-canvas
    /// double-clicks when an edge is selected (the double-click is
    /// the user re-confirming the edge, not asking for a node) or
    /// when `CreateOrphanNodeAndEdit` is unbound — which is the
    /// shipped default.
    Nothing,
}

/// What the caller still has to do after
/// [`apply_double_click_activate`] returns.
///
/// Exists because exactly one branch of the double-click behavior
/// needs state only native has (the single-line editor). Returning
/// the residual — rather than cfg-gating inside the core — keeps one
/// body for the parts both targets share and names the one part they
/// do not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::application::app) enum DoubleClickResidual {
    /// The core ran the whole behavior; nothing is left over.
    Done,
    /// The edge-label selection is committed and the scene is
    /// rebuilt. Native additionally opens the single-line editor on
    /// the clicked label; WASM has no single-line editor yet, so the
    /// browser stops at "the label is selected".
    ///
    /// Carries no payload on purpose. The identity is already in the
    /// caller's `DispatchHit`, and both consumers recover it through
    /// [`edge_label_target`] — the single conversion this module
    /// exists to keep them agreeing on. A copy threaded through here
    /// would be a second source for the same fact, readable only by
    /// the consumer that needs it least.
    OpenEdgeLabelEditor,
}

/// The [`DispatchOutcome`](super::DispatchOutcome) a
/// `DoubleClickActivate` step reports, from the residual
/// [`apply_double_click_activate`] produced. `None` means the step
/// never reached the apply half — there was no [`DispatchHit`] to
/// route on, so nothing was decided and nothing ran, which is the
/// shape every macro step has today.
///
/// **This is the discriminator, and it is written exactly once.** Two
/// callers must not disagree about whether a double-click did
/// anything: the dispatcher arm returns this value, and
/// `action_core::lift_mixed_branch_for_wasm_macro` reads it to decide
/// whether a WASM macro's `any_ran` flag bumps. The `match` is
/// exhaustive over the residual on purpose — a new
/// [`DoubleClickResidual`] variant is a build error here rather than
/// a gesture silently reported as work it did not do. Pure `Option`
/// shape in, enum out: no renderer, so `cargo test` pins every case
/// (`TEST_CONVENTIONS §T9`).
///
/// Note what `OpenEdgeLabelEditor` does **not** say: that work ran.
/// [`apply_double_click_activate`] skips the selection commit and the
/// rebuild when the label is already selected
/// ([`edge_label_selection_is_current`]) — the normal case, since the
/// double-click's first click has usually committed it already — and
/// still returns that residual, because native still has an editor to
/// open. So it maps to `Unhandled`: "this target has nothing to
/// report". Native's fall-through opens the editor and reports
/// `Handled` from there. Keying the decision on "was there a hit"
/// instead reports the already-selected case as work.
pub(in crate::application::app) fn double_click_outcome(
    residual: Option<DoubleClickResidual>,
) -> super::DispatchOutcome {
    match residual {
        None => super::DispatchOutcome::Unhandled,
        Some(DoubleClickResidual::Done) => super::DispatchOutcome::Handled,
        Some(DoubleClickResidual::OpenEdgeLabelEditor) => super::DispatchOutcome::Unhandled,
    }
}

/// Which of the two meanings an `Unhandled` outcome from a *pointer*
/// dispatch has, for the browser's reporting split.
///
/// Both arrive at the same place in `run_wasm`'s mouse handlers and
/// they are not the same event: one is a documented stop, the other
/// is a user's binding doing nothing at all.
#[derive(Debug, Clone, PartialEq, Eq)]
// Consumer is `run_wasm/`; the host build reaches this only from
// `#[cfg(test)]`, so arm the lint on wasm32 and silence it here
// rather than blanket-allowing on both targets — if the browser
// caller goes away, wasm32 says so.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(in crate::application::app) enum UnhandledPointerDispatch {
    /// The sanctioned carve-out: `DoubleClickActivate` on an edge
    /// label. The selection is committed (or was already current) and
    /// only the native single-line editor open is missing. Carries
    /// the `EdgeRef` for the `debug!` line.
    EdgeLabelEditorIsNativeOnly(EdgeRef),
    /// A `NativeOnly` Action the user bound to a pointer gesture,
    /// with no browser body at all — the gesture is a silent no-op
    /// and the user needs telling once.
    NativeOnlyBinding,
}

/// Classify an `Unhandled` pointer dispatch for the browser's
/// warn-vs-debug split.
///
/// Keyed on the **Action as well as the hit**, because the hit alone
/// cannot tell the two apart: binding a `NativeOnly` Action such as
/// `OpenConsole` to the double-click and using it on an edge label
/// produces an `Unhandled` with an `EdgeLabel` hit and *no*
/// edge-label carve-out — nothing ran, and reporting it as the
/// carve-out would file a dead binding as a sanctioned stop at a log
/// level release builds filter out.
///
/// `DoubleClickActivate` + an edge-label hit is the carve-out and
/// nothing else is: that pair is the only way
/// [`apply_double_click_activate`] can return
/// [`DoubleClickResidual::OpenEdgeLabelEditor`], which is in turn the
/// only `Unhandled` that arm produces with a hit present.
// Consumer is `run_wasm/`; the host build reaches this only from
// `#[cfg(test)]`, so arm the lint on wasm32 and silence it here
// rather than blanket-allowing on both targets — if the browser
// caller goes away, wasm32 says so.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(in crate::application::app) fn classify_unhandled_pointer_dispatch(
    action: &Action,
    click_hit: &ClickHit,
) -> UnhandledPointerDispatch {
    match (action, edge_label_target(click_hit)) {
        (Action::DoubleClickActivate, Some(edge_ref)) => {
            UnhandledPointerDispatch::EdgeLabelEditorIsNativeOnly(edge_ref)
        }
        _ => UnhandledPointerDispatch::NativeOnlyBinding,
    }
}

/// The `EdgeRef` identity a `ClickHit::EdgeLabel` names, if the hit
/// is one. Single source for the `EdgeKey` → `EdgeRef` conversion so
/// the route resolver and the native residual cannot disagree about
/// which edge the double-click meant.
pub(in crate::application::app) fn edge_label_target(click_hit: &ClickHit) -> Option<EdgeRef> {
    match click_hit {
        ClickHit::EdgeLabel(key) => Some(EdgeRef::new(
            key.from_id.as_str(),
            key.to_id.as_str(),
            key.edge_type.as_str(),
        )),
        _ => None,
    }
}

/// Is the current selection already exactly this edge label?
///
/// The double-click's first click has normally committed the
/// selection already, so re-committing it and rebuilding the scene
/// is work with no visible result. Pinned as a predicate rather than
/// written inline because it is the one place the two pre-unification
/// implementations disagreed, and §4's mobile budget makes the skip
/// worth keeping rather than smoothing away.
pub(in crate::application::app) fn edge_label_selection_is_current(
    selection: &SelectionState,
    edge_ref: &EdgeRef,
) -> bool {
    matches!(selection, SelectionState::EdgeLabel(s) if &s.edge_ref == edge_ref)
}

/// Decide what a double-click on `click_hit` should do.
///
/// Pure: takes the press hit, the current selection, and the
/// resolved keybind table; returns the route. No renderer, no tree,
/// no mutation — so every branch is reachable from `cargo test`.
pub(in crate::application::app) fn resolve_double_click_route(
    click_hit: &ClickHit,
    selection: &SelectionState,
    keybinds: &ResolvedKeybinds,
) -> DoubleClickRoute {
    match click_hit {
        ClickHit::Node(node_id, section_idx) => DoubleClickRoute::OpenNodeEditor {
            node_id: node_id.clone(),
            section_idx: *section_idx,
        },
        ClickHit::PortalMarker { edge, endpoint } | ClickHit::PortalText { edge, endpoint } => {
            let partner_id = if *endpoint == edge.from_id {
                edge.to_id.clone()
            } else {
                edge.from_id.clone()
            };
            DoubleClickRoute::PanToPortalPartner {
                edge: edge.clone(),
                partner_id,
            }
        }
        ClickHit::EdgeLabel(_) => match edge_label_target(click_hit) {
            Some(edge_ref) => DoubleClickRoute::EditEdgeLabel { edge_ref },
            // Unreachable: the arm already matched `EdgeLabel`.
            // Kept total rather than `expect`-ing, per §9 (no panics
            // on interactive paths).
            None => DoubleClickRoute::Nothing,
        },
        ClickHit::Empty => {
            // Empty-canvas double-click only creates when the user
            // opted in by binding `CreateOrphanNodeAndEdit`
            // somewhere — the gesture ships unbound. An edge
            // selection suppresses it outright: the user is working
            // on the edge, not asking for a node.
            let edge_selected = matches!(selection, SelectionState::Edge(_));
            if !edge_selected && keybinds.has_any_binding_for(Action::CreateOrphanNodeAndEdit) {
                DoubleClickRoute::CreateOrphanAndEdit
            } else {
                DoubleClickRoute::Nothing
            }
        }
    }
}

/// Run the double-click behavior for `hit`. The single body both
/// targets' `Action::DoubleClickActivate` dispatch reaches.
///
/// No-ops (returning [`DoubleClickResidual::Done`]) when no document
/// is loaded — every branch needs one, and an interactive path must
/// degrade rather than panic (§9).
pub(in crate::application::app) fn apply_double_click_activate(
    hit: &DispatchHit,
    core: &mut InputContextCore<'_>,
) -> DoubleClickResidual {
    let route = {
        let Some(doc) = core.document.as_deref() else {
            return DoubleClickResidual::Done;
        };
        resolve_double_click_route(&hit.click_hit, &doc.selection, core.keybinds)
    };
    match route {
        DoubleClickRoute::Nothing => DoubleClickResidual::Done,
        DoubleClickRoute::OpenNodeEditor { node_id, section_idx } => {
            if let Some(doc) = core.document.as_deref_mut() {
                // Section identity is preserved so the editor opens
                // on the section the user pointed at; collapsing to
                // `Single` here would send `open_text_edit` to
                // section 0 on every multi-section node.
                doc.selection = match section_idx {
                    Some(idx) => SelectionState::Section(SectionSel {
                        node_id: node_id.clone(),
                        section_idx: idx,
                    }),
                    None => SelectionState::Single(node_id.clone()),
                };
                rebuild_all(
                    doc,
                    core.interaction_mode,
                    core.mindmap_tree,
                    core.app_scene,
                    core.renderer,
                    core.scene_cache,
                );
                crate::application::app::text_edit::open_text_edit(
                    &node_id,
                    false,
                    doc,
                    core.text_edit_state,
                    core.mindmap_tree,
                    core.renderer,
                );
            }
            DoubleClickResidual::Done
        }
        DoubleClickRoute::PanToPortalPartner { edge, partner_id } => {
            // Camera first, then selection + rebuild — the order the
            // scene rebuild depends on, since the connection pass
            // projects against the current camera.
            if let Some(doc) = core.document.as_deref() {
                if let Some(node) = doc.mindmap.nodes.get(&partner_id) {
                    core.renderer.set_camera_center(node.center_vec2());
                }
            }
            if let Some(doc) = core.document.as_deref_mut() {
                doc.selection =
                    SelectionState::Edge(EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type));
                rebuild_all(
                    doc,
                    core.interaction_mode,
                    core.mindmap_tree,
                    core.app_scene,
                    core.renderer,
                    core.scene_cache,
                );
            }
            DoubleClickResidual::Done
        }
        DoubleClickRoute::EditEdgeLabel { edge_ref } => {
            if let Some(doc) = core.document.as_deref_mut() {
                if !edge_label_selection_is_current(&doc.selection, &edge_ref) {
                    let prev = doc.selection.clone();
                    doc.selection = SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref.clone()));
                    rebuild_after_selection_change(&prev, &mut super::rebuild_ctx!(core, doc));
                }
            }
            DoubleClickResidual::OpenEdgeLabelEditor
        }
        DoubleClickRoute::CreateOrphanAndEdit => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::rebuild_ctx!(core, doc);
                apply_create_orphan_node_and_edit(hit.canvas_pos, &mut rc, core.text_edit_state);
            }
            DoubleClickResidual::Done
        }
    }
}

/// winit's touch phase in the recognizer's stable vocabulary.
///
/// `Cancelled` folds into `Ended`: from the recognizer's point of
/// view a canceled finger and a lifted finger both free the slot,
/// and the difference is invisible to gesture recognition. Both
/// targets translated this identically at their own boundary; it is
/// here so a fourth winit phase cannot be handled on one target and
/// forgotten on the other.
pub(in crate::application::app) fn touch_phase(phase: winit::event::TouchPhase) -> Phase {
    match phase {
        winit::event::TouchPhase::Started => Phase::Started,
        winit::event::TouchPhase::Moved => Phase::Moved,
        winit::event::TouchPhase::Ended => Phase::Ended,
        winit::event::TouchPhase::Cancelled => Phase::Cancelled,
    }
}

/// A recognized touch gesture that resolves through the keybind
/// table, already looked up.
///
/// Carried by [`TouchStep::Dispatch`]. The caller performs the
/// dispatch, because that is the part the two targets genuinely do
/// differently: native goes through `dispatch_action` with the full
/// native context, the browser through `dispatch_compatible` plus a
/// `NativeOnly` warn-log.
#[derive(Debug)]
pub(in crate::application::app) struct TouchGestureDispatch {
    /// Where the gesture happened. The caller assigns this to
    /// `cursor_pos` **before** dispatching, so the Action reads the
    /// position the finger was at rather than wherever the mouse
    /// cursor last was.
    pub(in crate::application::app) cursor_pos: (f64, f64),
    /// Canonical gesture key name (`"longpress"`) — what the lookup
    /// used, and what the browser's warn-log names. Only the browser
    /// reads it: native's own handlers already hold the name they
    /// looked the binding up with.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(in crate::application::app) gesture_name: &'static str,
    /// The bound Action, or `None` when the user has no binding for
    /// this gesture. `None` still yields a `TouchGestureDispatch`
    /// rather than collapsing to `None` at the outer level: both
    /// targets moved the cursor as soon as a gesture was
    /// *recognized*, before ever consulting the table, and folding
    /// the two cases together would silently drop that move.
    pub(in crate::application::app) action: Option<Action>,
}

/// A recognized touch gesture that resolves to no `Action` at all,
/// and to one body [`apply_touch_effect`] runs on both targets.
///
/// Each variant takes a carve-out `CODE_CONVENTIONS §3` already
/// grants the mouse: [`TouchEffect::TapSelect`] is the pre-funnel
/// selection bookkeeping a single left-click runs before the funnel,
/// and [`TouchEffect::CameraStep`] is the per-frame
/// continuous-gesture body a left-drag's per-cursor-move delta runs
/// outside it.
///
/// **This is why touch pan and pinch need no `Action`, and could not
/// have used one.** The obvious wiring, `Action::PanCanvas`, does not
/// move the camera: it *arms* `DragState::Panning` (see
/// `dispatch::native`'s `route_pan_canvas`), a state that exists only
/// on native. Dispatching it from the browser returns
/// `DispatchOutcome::Unhandled` and warns — which is the defect this
/// vocabulary exists to remove, reproduced one layer down. The camera
/// itself is moved by `RenderDecree`s, which are cross-platform, so
/// the gesture reaches it directly and no `Action` classification
/// changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::application::app) enum TouchEffect {
    /// Commit the selection under the finger — the touch peer of a
    /// left click. `screen_pos` is in physical pixels, the space the
    /// recognizer works in.
    TapSelect { screen_pos: (f64, f64) },
    /// One step of a continuous camera gesture: translate by `pan`,
    /// then scale by `scale` about `anchor`. A one-finger pan is the
    /// degenerate case with `scale == 1.0`; a two-finger step carries
    /// both halves, because two fingers moving describe a translation
    /// and a scale at once and applying only one of them would make
    /// the canvas slide out from under them.
    CameraStep {
        /// Screen-space point the scale is applied about — the
        /// finger for a pan, the midpoint of the pair for a pinch.
        anchor: (f64, f64),
        /// Screen-space translation this step asks for.
        pan: (f64, f64),
        /// Multiplicative zoom factor for this step; `1.0` for a
        /// gesture that is pure translation.
        scale: f64,
    },
}

/// What one touch event asks the runtime to do — the two routes a
/// [`RecognizedGesture`] can take, named so a runtime handles both.
#[derive(Debug)]
pub(in crate::application::app) enum TouchStep {
    /// Run the `Action` the keybind table gave this gesture.
    Dispatch(TouchGestureDispatch),
    /// Run [`apply_touch_effect`] — the same body on both targets.
    Effect(TouchEffect),
}

/// Feed one touch event through the recognizer and turn whatever it
/// recognizes into the step the runtime owes.
///
/// The whole of the ingest → tick → route sequence both runtimes ran
/// as a near-copy. Returns `None` when the event drove no
/// recognition — the caller then falls back to "redraw on
/// Started/Moved" so cursor-following gesture chrome still updates.
///
/// The `match` over [`RecognizedGesture`] is exhaustive on purpose:
/// a new gesture in the recognizer's vocabulary is a build error here
/// rather than an emission nothing consumes.
///
/// Modifiers are fixed all-false at the keybind lookup: touch devices
/// have no modifier keys, and the `LongPress` binding doesn't carry
/// Ctrl/Shift/Alt either.
///
/// Either `ingest` or `tick` can produce at most one recognition per
/// call. When both fire, ingest wins — it is the more recent
/// transition — and the tick's emission is picked up on the next
/// call. Both are always run: `tick` must not be short-circuited by
/// an ingest hit, or the long-press timer would stall.
///
/// The "both fire" case is **not reachable** in the current
/// recognizer, so the `or` ordering is unobservable and no test pins
/// it: `tick` emits only from a `OneFinger` state that has neither
/// moved nor spent its discrete emission, and every `ingest` emission
/// either leaves that state (`Tap`) or requires the condition `tick`
/// tests to be false (`Pan` needs movement; `PinchStep` needs two
/// fingers). The order is written down anyway because it is the
/// answer the moment those stop being disjoint, and getting it wrong
/// then would silently delay a gesture by one event rather than fail
/// loudly.
pub(in crate::application::app) fn drive_touch_event(
    recognizer: &mut TouchGestureRecognizer,
    keybinds: &ResolvedKeybinds,
    phase: Phase,
    id: u64,
    pos: (f64, f64),
    now: web_time::Instant,
) -> Option<TouchStep> {
    let from_ingest = recognizer.ingest(phase, id, pos, now);
    let from_tick = recognizer.tick(now);
    Some(match from_ingest.or(from_tick)? {
        RecognizedGesture::LongPress { pos } => {
            let gesture_name = MouseGesture::LongPress.key_name();
            TouchStep::Dispatch(TouchGestureDispatch {
                cursor_pos: pos,
                gesture_name,
                action: keybinds.action_for_gesture(gesture_name, false, false, false),
            })
        }
        RecognizedGesture::Tap { pos } => TouchStep::Effect(TouchEffect::TapSelect { screen_pos: pos }),
        RecognizedGesture::Pan { pos, delta } => TouchStep::Effect(TouchEffect::CameraStep {
            anchor: pos,
            pan: delta,
            scale: 1.0,
        }),
        RecognizedGesture::PinchStep { center, pan, scale } => TouchStep::Effect(TouchEffect::CameraStep {
            anchor: center,
            pan,
            scale,
        }),
    })
}

/// Run a [`TouchEffect`] against the cross-platform dispatch context.
///
/// One body, both targets: everything either effect touches — the
/// hit-test chain, the selection, the rebuild tier, the camera
/// decrees — is already cross-platform, so there is nothing here for
/// a `cfg` to select between.
pub(in crate::application::app) fn apply_touch_effect(effect: TouchEffect, core: &mut InputContextCore<'_>) {
    // The finger is the pointer now. Every later Action that reads
    // `cursor_pos` — a keyboard `CreateOrphanNode`, a macro step —
    // should place itself where the user last touched, not where a
    // mouse was left.
    match effect {
        TouchEffect::TapSelect { screen_pos } => {
            *core.cursor_pos = screen_pos;
            apply_tap_select(screen_pos, core);
        }
        TouchEffect::CameraStep { anchor, pan, scale } => {
            *core.cursor_pos = anchor;
            let (translate, zoom) = camera_step_decrees(anchor, pan, scale);
            core.renderer.process_decree(translate);
            if let Some(zoom) = zoom {
                core.renderer.process_decree(zoom);
            }
        }
    }
}

/// The selection a pointer commit lands on once the hit chain has
/// come up empty: the connection path under the pointer, or nothing.
///
/// The last rung of the ladder, and the one `compute_click_hit`
/// cannot answer — a connection path is a curve, not an AABB, so it
/// is hit with a tolerance in canvas units rather than through the
/// scene's bounding-volume descent. `canvas_per_pixel` is the
/// renderer's, passed as a plain value so the rung has no renderer
/// in it.
///
/// Shared rather than inlined because three pointer paths reach it
/// and two of them used to disagree: native's click router
/// (`click::handle_click_core`) has always run it, the browser's
/// click release never did, and the touch tap needs it on both
/// targets. Before this the browser could *tap* a connection path
/// and select it while a *mouse click* on the same path selected
/// nothing (CODE_CONVENTIONS §4 — the two are peers).
pub(in crate::application::app) fn edge_under_pointer(
    canvas_pos: Vec2,
    map: &baumhard::mindmap::model::MindMap,
    canvas_per_pixel: CanvasPerPixel,
) -> SelectionState {
    match hit_test_edge(canvas_pos, map, canvas_per_pixel.scale(EDGE_HIT_TOLERANCE_PX)) {
        Some(edge_ref) => SelectionState::Edge(edge_ref),
        None => SelectionState::None,
    }
}

/// The decrees one [`TouchEffect::CameraStep`] asks the renderer for,
/// in order: always a translation, and a zoom only when the step
/// actually scaled.
///
/// Split out of [`apply_touch_effect`] because it is the half with a
/// rule in it, and the half a test can reach: `apply_touch_effect`
/// drives a live `Renderer`, which `TEST_CONVENTIONS §T8` keeps out
/// of the harness, while *which decrees a step asks for* is plain
/// values in and plain values out.
///
/// **The rule: a step that only panned must not emit `CameraZoom`.**
/// `CameraZoom` is the only decree that raises the renderer's
/// connection-geometry dirty flag (`renderer/decree.rs`), and the
/// flag costs a scene reprojection on the next frame — on native
/// through `drain_camera_geometry_rebuild`, in the browser through
/// `WasmInputState::reproject_after_camera_change`. A one-finger pan
/// changes no effective font size and no sample spacing, so paying
/// for one would be work §4's mobile budget did not ask for.
///
/// The `!= 1.0` is an exact float compare, and it is the right one:
/// `1.0` is a sentinel this crate writes — a one-finger pan, or a
/// pinch step whose baseline separation was too small to form a ratio
/// against — not a measured quantity that might land near it.
fn camera_step_decrees(
    anchor: (f64, f64),
    pan: (f64, f64),
    scale: f64,
) -> (RenderDecree, Option<RenderDecree>) {
    let translate = RenderDecree::CameraPan(pan.0 as f32, pan.1 as f32);
    let zoom = (scale != 1.0).then_some(RenderDecree::CameraZoom {
        screen_x: anchor.0 as f32,
        screen_y: anchor.1 as f32,
        factor: scale as f32,
    });
    (translate, zoom)
}

/// The tap's body: resolve what is under the finger through the same
/// hit chain a click runs, fire that node's `OnClick` triggers, commit
/// the selection, and rebuild at the tier the outcome earns.
///
/// The sequence — triggers, then the pre-write selection snapshot,
/// then the write, then [`RebuildTier::for_click`] — is the one
/// `click::handle_click_core` runs, and for the same reasons: a
/// document action a trigger performs (a theme switch) must land
/// before the rebuild reads the document, and the tier is a function
/// of how the selection *moved*, so it cannot be derived after the
/// write.
///
/// No-ops before the first document loads.
fn apply_tap_select(screen_pos: (f64, f64), core: &mut InputContextCore<'_>) {
    let Some(doc) = core.document.as_deref_mut() else {
        return;
    };
    let canvas_pos = core
        .renderer
        .screen_to_canvas(screen_pos.0 as f32, screen_pos.1 as f32);
    let ClickHitParts {
        hit_node,
        hit_section_idx,
        portal_text_hit,
        portal_icon_hit,
        edge_label_hit,
        ..
    } = compute_click_hit(canvas_pos, core.mindmap_tree.as_mut(), core.app_scene);

    let triggers_fired = match hit_node.as_ref() {
        Some(id) => fire_onclick_triggers(
            doc,
            core.mindmap_tree,
            core.scene_cache,
            id,
            hit_section_idx,
            PLATFORM_CONTEXT,
            now_ms() as u64,
        ),
        None => false,
    };

    let prev_selection = doc.selection.clone();
    doc.selection = if let Some(id) = hit_node {
        // Shift is fixed false — a finger carries no modifiers — so
        // this is the plain "select what I touched" branch, section-
        // aware exactly as a click is.
        compute_node_click_selection(&doc.selection, &id, hit_section_idx, false, core.interaction_mode)
    } else if let Some((edge_key, endpoint_node_id)) = portal_text_hit {
        SelectionState::PortalText(PortalLabelSel {
            edge_key,
            endpoint_node_id,
        })
    } else if let Some((edge_key, endpoint_node_id)) = portal_icon_hit {
        SelectionState::PortalLabel(PortalLabelSel {
            edge_key,
            endpoint_node_id,
        })
    } else if let Some(key) = edge_label_hit {
        SelectionState::EdgeLabel(EdgeLabelSel::new(EdgeRef::new(
            key.from_id.as_str(),
            key.to_id.as_str(),
            key.edge_type.as_str(),
        )))
    } else {
        edge_under_pointer(canvas_pos, &doc.mindmap, CanvasPerPixel::of(core.renderer))
    };

    RebuildTier::for_click(triggers_fired, &prev_selection, &doc.selection)
        .execute(&mut super::rebuild_ctx!(core, doc));
}

/// Pure selection-update helper for "click landed on a node."
///
/// Resolves the new [`SelectionState`] given the previous selection,
/// the click hit (node id + optional section index), the shift modifier,
/// and the current [`InteractionMode`]. Section routing is gated by
/// [`InteractionMode::click_resolves_to_section`]: outside `NodeEdit { id }`
/// (or in NodeEdit on a different node) every click on a multi-section
/// node folds to whole-node `Single` / `Multi`. Single-section nodes
/// always fold via `hit_test_target`'s short-circuit (they never
/// produce `hit_section = Some(_)`), so their click behavior is
/// unchanged from pre-Batch-3.
///
/// Plain click:
/// - `route_to_section` true → `Section { node_id, section_idx }`.
/// - else → `Single(node_id)`.
///
/// Shift+click, section-routed:
/// - `Section(s)` matching the new (node, idx) → `None` (toggle off).
/// - `Section(s)` mismatching → promote to `MultiSection`.
/// - `MultiSection` → toggle the (node, idx) pair in or out, narrowing
///   back to `Section` when one remains.
/// - any non-section starting state → start a fresh `Section`.
///
/// Shift+click, whole-node (route_to_section false):
/// - `Single(existing)` matching → `None` (toggle off).
/// - `Single(existing)` mismatching → `Multi(vec![existing, new])`.
/// - `Multi` → toggle id in or out, narrowing back to `Single`.
/// - any non-node starting state → fresh `Single`.
pub(in crate::application::app) fn compute_node_click_selection(
    existing: &SelectionState,
    hit_id: &str,
    hit_section: Option<usize>,
    shift_pressed: bool,
    interaction_mode: &InteractionMode,
) -> SelectionState {
    // The routing decision and the value it routes are one thing, so
    // they are bound together: an `is_some()` test followed by a
    // re-`expect` further down is two chances for the two to drift.
    let routed_section = hit_section.filter(|_| interaction_mode.click_resolves_to_section(hit_id));

    if !shift_pressed {
        return match routed_section {
            Some(section_idx) => SelectionState::Section(SectionSel {
                node_id: hit_id.to_string(),
                section_idx,
            }),
            None => SelectionState::Single(hit_id.to_string()),
        };
    }

    if let Some(section_idx) = routed_section {
        let new_sec = SectionSel {
            node_id: hit_id.to_string(),
            section_idx,
        };
        return match existing {
            SelectionState::Section(prev) if prev == &new_sec => SelectionState::None,
            SelectionState::Section(prev) => SelectionState::MultiSection(vec![prev.clone(), new_sec]),
            SelectionState::MultiSection(prev) => {
                let mut secs = prev.clone();
                if let Some(pos) = secs.iter().position(|s| s == &new_sec) {
                    secs.remove(pos);
                    SelectionState::from_sections(secs)
                } else {
                    secs.push(new_sec);
                    SelectionState::MultiSection(secs)
                }
            }
            _ => SelectionState::Section(new_sec),
        };
    }

    // Whole-node shift+click: existing behavior (toggle node in/out of Multi).
    match existing {
        SelectionState::None
        | SelectionState::Edge(_)
        | SelectionState::EdgeLabel(_)
        | SelectionState::PortalLabel(_)
        | SelectionState::PortalText(_)
        | SelectionState::Section(_)
        | SelectionState::MultiSection(_)
        | SelectionState::SectionRange { .. } => SelectionState::Single(hit_id.to_string()),
        SelectionState::Single(prev) => {
            if prev == hit_id {
                SelectionState::None
            } else {
                SelectionState::Multi(vec![prev.clone(), hit_id.to_string()])
            }
        }
        SelectionState::Multi(prev) => {
            let mut ids = prev.clone();
            if let Some(pos) = ids.iter().position(|i| i == hit_id) {
                ids.remove(pos);
                SelectionState::from_ids(ids)
            } else {
                ids.push(hit_id.to_string());
                SelectionState::Multi(ids)
            }
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    //! Route resolution is the half of the double-click behavior
    //! that differed between the two pre-unification
    //! implementations, and it is the half that runs without a
    //! renderer — so it carries the coverage. The apply half is a
    //! renderer-driving `match` over these routes and is out of
    //! scope per `TEST_CONVENTIONS §T8`.

    use super::*;
    use crate::application::document::SectionSel;
    use crate::application::keybinds::KeybindConfig;

    fn keybinds_with_orphan_edit_bound() -> ResolvedKeybinds {
        KeybindConfig {
            create_orphan_node_and_edit: vec!["Ctrl+Shift+N".into()],
            ..Default::default()
        }
        .resolve()
    }

    fn keybinds_default() -> ResolvedKeybinds {
        KeybindConfig::default().resolve()
    }

    #[test]
    fn test_double_click_route_node_without_section_opens_whole_node() {
        let route = resolve_double_click_route(
            &ClickHit::Node("n1".into(), None),
            &SelectionState::None,
            &keybinds_default(),
        );
        assert_eq!(
            route,
            DoubleClickRoute::OpenNodeEditor {
                node_id: "n1".into(),
                section_idx: None,
            }
        );
    }

    #[test]
    fn test_double_click_route_node_preserves_section_index() {
        let route = resolve_double_click_route(
            &ClickHit::Node("n1".into(), Some(2)),
            &SelectionState::None,
            &keybinds_default(),
        );
        assert_eq!(
            route,
            DoubleClickRoute::OpenNodeEditor {
                node_id: "n1".into(),
                section_idx: Some(2),
            }
        );
    }

    #[test]
    fn test_double_click_route_portal_icon_targets_the_far_endpoint() {
        let route = resolve_double_click_route(
            &ClickHit::PortalMarker {
                edge: EdgeKey::new("a", "b", "cross_link"),
                endpoint: "a".into(),
            },
            &SelectionState::None,
            &keybinds_default(),
        );
        assert_eq!(
            route,
            DoubleClickRoute::PanToPortalPartner {
                edge: EdgeKey::new("a", "b", "cross_link"),
                partner_id: "b".into(),
            }
        );
    }

    /// Standing on the `to` endpoint navigates back to `from` — the
    /// partner is "whichever end I am not on", not "always `to`".
    #[test]
    fn test_double_click_route_portal_icon_from_far_end_targets_the_near_endpoint() {
        let route = resolve_double_click_route(
            &ClickHit::PortalMarker {
                edge: EdgeKey::new("a", "b", "cross_link"),
                endpoint: "b".into(),
            },
            &SelectionState::None,
            &keybinds_default(),
        );
        assert_eq!(
            route,
            DoubleClickRoute::PanToPortalPartner {
                edge: EdgeKey::new("a", "b", "cross_link"),
                partner_id: "a".into(),
            }
        );
    }

    /// Portal text and portal icon share one route: same endpoint
    /// identity, same navigate intent.
    #[test]
    fn test_double_click_route_portal_text_matches_portal_icon() {
        let icon = resolve_double_click_route(
            &ClickHit::PortalMarker {
                edge: EdgeKey::new("a", "b", "cross_link"),
                endpoint: "a".into(),
            },
            &SelectionState::None,
            &keybinds_default(),
        );
        let text = resolve_double_click_route(
            &ClickHit::PortalText {
                edge: EdgeKey::new("a", "b", "cross_link"),
                endpoint: "a".into(),
            },
            &SelectionState::None,
            &keybinds_default(),
        );
        assert_eq!(icon, text);
    }

    #[test]
    fn test_double_click_route_edge_label_carries_the_edge_ref() {
        let route = resolve_double_click_route(
            &ClickHit::EdgeLabel(EdgeKey::new("a", "b", "cross_link")),
            &SelectionState::None,
            &keybinds_default(),
        );
        assert_eq!(
            route,
            DoubleClickRoute::EditEdgeLabel {
                edge_ref: EdgeRef::new("a", "b", "cross_link"),
            }
        );
    }

    /// Shipped default: empty-canvas double-click does nothing,
    /// because `CreateOrphanNodeAndEdit` is unbound out of the box.
    #[test]
    fn test_double_click_route_empty_canvas_is_a_no_op_when_unbound() {
        let route = resolve_double_click_route(&ClickHit::Empty, &SelectionState::None, &keybinds_default());
        assert_eq!(route, DoubleClickRoute::Nothing);
    }

    #[test]
    fn test_double_click_route_empty_canvas_creates_when_bound() {
        let route = resolve_double_click_route(
            &ClickHit::Empty,
            &SelectionState::None,
            &keybinds_with_orphan_edit_bound(),
        );
        assert_eq!(route, DoubleClickRoute::CreateOrphanAndEdit);
    }

    /// An edge selection suppresses the create even when the Action
    /// is bound — the user is working on the edge.
    #[test]
    fn test_double_click_route_empty_canvas_suppressed_by_edge_selection() {
        let route = resolve_double_click_route(
            &ClickHit::Empty,
            &SelectionState::Edge(EdgeRef::new("a", "b", "cross_link")),
            &keybinds_with_orphan_edit_bound(),
        );
        assert_eq!(route, DoubleClickRoute::Nothing);
    }

    /// Only an *edge* selection suppresses it. An edge-label or a
    /// node selection does not — pinned so a future widening of the
    /// guard to "anything edge-adjacent" is a visible diff.
    #[test]
    fn test_double_click_route_empty_canvas_not_suppressed_by_edge_label_selection() {
        let route = resolve_double_click_route(
            &ClickHit::Empty,
            &SelectionState::EdgeLabel(EdgeLabelSel::new(EdgeRef::new("a", "b", "cross_link"))),
            &keybinds_with_orphan_edit_bound(),
        );
        assert_eq!(route, DoubleClickRoute::CreateOrphanAndEdit);
    }

    #[test]
    fn test_edge_label_target_is_none_for_non_label_hits() {
        assert!(edge_label_target(&ClickHit::Empty).is_none());
        assert!(edge_label_target(&ClickHit::Node("n".into(), None)).is_none());
        assert!(edge_label_target(&ClickHit::PortalMarker {
            edge: EdgeKey::new("a", "b", "cross_link"),
            endpoint: "a".into(),
        })
        .is_none());
        assert_eq!(
            edge_label_target(&ClickHit::EdgeLabel(EdgeKey::new("a", "b", "cross_link"))),
            Some(EdgeRef::new("a", "b", "cross_link")),
        );
    }

    /// Edge identity is the *triple* `(from_id, to_id, edge_type)`,
    /// and every component carries the same weight:
    /// `format/validation.md` permits several edges between the same
    /// pair distinguished only by the type, and `EdgeRef::matches`
    /// compares all three. A conversion that dropped **any** one of
    /// them — substituting a constant for it, which is what a
    /// hardcoded fixture value hides — hands the native residual an
    /// `EdgeRef` naming a *different* edge, and the single-line
    /// editor silently opens on a label the user did not click.
    /// Precisely what [`edge_label_target`]'s rustdoc claims to
    /// prevent.
    ///
    /// Each case therefore varies exactly one component away from the
    /// baseline, so substituting the baseline value back in for that
    /// component fails here and nowhere else has to notice.
    #[test]
    fn test_edge_label_target_carries_all_three_identity_components() {
        for (from, to, edge_type) in [
            ("a", "b", "cross_link"),
            // `from_id` varied, other two at baseline.
            ("z", "b", "cross_link"),
            // `to_id` varied.
            ("a", "z", "cross_link"),
            // `edge_type` varied — three ways, including empty.
            ("a", "b", "hierarchy"),
            ("a", "b", "portal"),
            ("a", "b", ""),
            // All three away from the baseline at once.
            ("n1", "n2", "hierarchy"),
        ] {
            assert_eq!(
                edge_label_target(&ClickHit::EdgeLabel(EdgeKey::new(from, to, edge_type))),
                Some(EdgeRef::new(from, to, edge_type)),
                "({from:?}, {to:?}, {edge_type:?}) must survive the EdgeKey → EdgeRef conversion",
            );
        }
    }

    /// The same identity, one component at a time, through the full
    /// resolver rather than the helper — so the route the apply half
    /// receives is pinned too, not just the conversion. Two hits that
    /// differ in any single component are two different edges and so
    /// two different routes.
    #[test]
    fn test_double_click_route_edge_label_distinguishes_edges_by_each_component_alone() {
        let route_of = |from: &str, to: &str, edge_type: &str| {
            resolve_double_click_route(
                &ClickHit::EdgeLabel(EdgeKey::new(from, to, edge_type)),
                &SelectionState::None,
                &keybinds_default(),
            )
        };
        let baseline = route_of("a", "b", "cross_link");
        assert_eq!(
            baseline,
            DoubleClickRoute::EditEdgeLabel {
                edge_ref: EdgeRef::new("a", "b", "cross_link"),
            }
        );
        for (from, to, edge_type, component) in [
            ("z", "b", "cross_link", "from_id"),
            ("a", "z", "cross_link", "to_id"),
            ("a", "b", "hierarchy", "edge_type"),
        ] {
            let varied = route_of(from, to, edge_type);
            assert_eq!(
                varied,
                DoubleClickRoute::EditEdgeLabel {
                    edge_ref: EdgeRef::new(from, to, edge_type),
                },
                "{component} must reach the route verbatim",
            );
            assert_ne!(
                varied, baseline,
                "a hit differing only in {component} must resolve to a different edge",
            );
        }
    }

    #[test]
    fn test_edge_label_selection_is_current_matches_only_the_same_label() {
        let er = EdgeRef::new("a", "b", "cross_link");
        assert!(edge_label_selection_is_current(
            &SelectionState::EdgeLabel(EdgeLabelSel::new(er.clone())),
            &er
        ));
        assert!(!edge_label_selection_is_current(
            &SelectionState::EdgeLabel(EdgeLabelSel::new(EdgeRef::new("a", "c", "cross_link"))),
            &er
        ));
        // ... and differing in `from_id` alone, the third component
        // of the same triple.
        assert!(!edge_label_selection_is_current(
            &SelectionState::EdgeLabel(EdgeLabelSel::new(EdgeRef::new("z", "b", "cross_link"))),
            &er
        ));
        // Differing in `edge_type` *alone* is differing. Two edges
        // between the same pair are distinct edges when their types
        // differ (`format/validation.md`), so the committed selection
        // for one is not current for the other and the double-click
        // must re-commit rather than skip.
        assert!(!edge_label_selection_is_current(
            &SelectionState::EdgeLabel(EdgeLabelSel::new(EdgeRef::new("a", "b", "hierarchy"))),
            &er
        ));
        assert!(!edge_label_selection_is_current(
            &SelectionState::EdgeLabel(EdgeLabelSel::new(er.clone())),
            &EdgeRef::new("a", "b", "hierarchy")
        ));
        // The *edge* being selected is not the *label* being
        // selected — the double-click must still commit.
        assert!(!edge_label_selection_is_current(
            &SelectionState::Edge(er.clone()),
            &er
        ));
        assert!(!edge_label_selection_is_current(&SelectionState::None, &er));
        assert!(!edge_label_selection_is_current(
            &SelectionState::Section(SectionSel::new("n", 0)),
            &er
        ));
    }

    // -------------------------------------------------------------
    // Residual → outcome, and the browser's report split
    //
    // Both are `Option`-shape / enum in, enum out — the decisions the
    // dispatcher arm and the browser's mouse handler used to make
    // inline, where a `Renderer` (`TEST_CONVENTIONS §T8`) and a
    // `#[cfg(target_arch = "wasm32")]` module respectively put them
    // out of the suite's reach. §T9: platform-shared pure logic has
    // to be reachable without a wgpu instance.
    // -------------------------------------------------------------

    /// Every residual maps to exactly one outcome, and the `None`
    /// case — the hitless soft-skip a macro produces — is
    /// `Unhandled`, so the macro loop's `any_ran` does not bump for a
    /// step that touched nothing.
    #[test]
    fn test_double_click_outcome_maps_every_residual() {
        assert_eq!(
            double_click_outcome(None),
            crate::application::app::dispatch::DispatchOutcome::Unhandled
        );
        assert_eq!(
            double_click_outcome(Some(DoubleClickResidual::Done)),
            crate::application::app::dispatch::DispatchOutcome::Handled,
        );
        assert_eq!(
            double_click_outcome(Some(DoubleClickResidual::OpenEdgeLabelEditor)),
            crate::application::app::dispatch::DispatchOutcome::Unhandled,
        );
    }

    /// The three cases are genuinely three: no two of them collapse.
    /// A mapping that answered the same thing everywhere would pass
    /// two thirds of the test above.
    #[test]
    fn test_double_click_outcome_distinguishes_ran_from_did_not_run() {
        assert_ne!(
            double_click_outcome(Some(DoubleClickResidual::Done)),
            double_click_outcome(None),
        );
        assert_ne!(
            double_click_outcome(Some(DoubleClickResidual::Done)),
            double_click_outcome(Some(DoubleClickResidual::OpenEdgeLabelEditor)),
        );
    }

    /// The sanctioned carve-out is `DoubleClickActivate` on an edge
    /// label and nothing else.
    #[test]
    fn test_classify_unhandled_pointer_dispatch_names_the_edge_label_carve_out() {
        assert_eq!(
            classify_unhandled_pointer_dispatch(
                &Action::DoubleClickActivate,
                &ClickHit::EdgeLabel(EdgeKey::new("a", "b", "cross_link")),
            ),
            UnhandledPointerDispatch::EdgeLabelEditorIsNativeOnly(EdgeRef::new("a", "b", "cross_link")),
        );
    }

    /// **The case the hit alone gets wrong.** Bind a `NativeOnly`
    /// Action — `OpenConsole` is the example the browser's warn text
    /// names — to the double-click and use it on an edge label. The
    /// dispatcher returns `Unhandled` from the `NativeOnly` early
    /// return, having run nothing at all, while the *hit* is still an
    /// edge label. Classifying on the hit files a dead binding as the
    /// sanctioned carve-out and reports it at `debug!`, which the
    /// default filter drops: the user gets silence, which is the
    /// outcome BLK-1 rejected.
    #[test]
    fn test_classify_unhandled_pointer_dispatch_is_not_fooled_by_the_hit() {
        for action in [Action::OpenConsole, Action::EditSelection, Action::PanCanvas] {
            assert_eq!(
                classify_unhandled_pointer_dispatch(
                    &action,
                    &ClickHit::EdgeLabel(EdgeKey::new("a", "b", "cross_link")),
                ),
                UnhandledPointerDispatch::NativeOnlyBinding,
                "{action:?} bound to a pointer gesture is a dead binding, not the carve-out",
            );
        }
    }

    /// ... and `DoubleClickActivate` on anything that is not an edge
    /// label is not the carve-out either.
    #[test]
    fn test_classify_unhandled_pointer_dispatch_needs_an_edge_label_hit() {
        for hit in [
            ClickHit::Empty,
            ClickHit::Node("n1".into(), None),
            ClickHit::PortalMarker {
                edge: EdgeKey::new("a", "b", "cross_link"),
                endpoint: "a".into(),
            },
        ] {
            assert_eq!(
                classify_unhandled_pointer_dispatch(&Action::DoubleClickActivate, &hit),
                UnhandledPointerDispatch::NativeOnlyBinding,
                "{hit:?} is not the edge-label carve-out",
            );
        }
    }

    // -------------------------------------------------------------
    // Touch driving
    //
    // `drive_touch_event` is the ingest -> tick -> lookup sequence
    // both runtimes ran as a near-copy. It takes a recognizer and a
    // keybind table and returns plain values, so the whole thing is
    // reachable without an event loop.
    // -------------------------------------------------------------

    use crate::application::app::touch_gesture::TouchGestureRecognizer;
    use crate::application::keybinds::MouseGesture;
    use std::time::Duration;
    use web_time::Instant;

    /// Every winit phase maps to the recognizer's vocabulary, and
    /// **`Cancelled` keeps its own variant**. It used to fold onto
    /// `Ended`, which was harmless while no gesture fired on a lift;
    /// now that a deliberate lift is a tap, folding them would let
    /// the operating system commit a selection every time it
    /// interrupted a finger. `touch_phase` matches exhaustively, so
    /// a new winit variant is a build error rather than a phase
    /// silently handled on one target only.
    #[test]
    fn test_touch_phase_translates_every_winit_phase() {
        use winit::event::TouchPhase as W;
        assert_eq!(touch_phase(W::Started), Phase::Started);
        assert_eq!(touch_phase(W::Moved), Phase::Moved);
        assert_eq!(touch_phase(W::Ended), Phase::Ended);
        assert_eq!(touch_phase(W::Cancelled), Phase::Cancelled);
    }

    /// The [`TouchGestureDispatch`] a step carries, or a panic naming
    /// what arrived instead.
    fn dispatched(step: Option<TouchStep>) -> TouchGestureDispatch {
        match step {
            Some(TouchStep::Dispatch(d)) => d,
            other => panic!("expected a keybind-routed gesture, got {other:?}"),
        }
    }

    /// The [`TouchEffect`] a step carries, or a panic naming what
    /// arrived instead.
    fn effect_of(step: Option<TouchStep>) -> TouchEffect {
        match step {
            Some(TouchStep::Effect(e)) => e,
            other => panic!("expected a direct-effect gesture, got {other:?}"),
        }
    }

    /// A config with **every** gesture-defaulted binding cleared —
    /// the whole set `KeybindConfig::default()` ships pointing at a
    /// `MouseGesture`, not a convenient subset.
    ///
    /// The control for "this route does not consult the table":
    /// under it anything resolving through `action_for_gesture`
    /// comes back `None`, so only the routes that never ask still
    /// work. `double_click_activate` is in the list because the doc
    /// says *every*: touch never looks `"doubleclick"` up, so
    /// leaving it bound would have been harmless and the sentence
    /// would still have been false.
    fn keybinds_with_no_gesture_bindings() -> ResolvedKeybinds {
        KeybindConfig {
            enter_resize_mode: vec![],
            fast_resize_start: vec![],
            pan_canvas: vec![],
            zoom_in: vec![],
            zoom_out: vec![],
            double_click_activate: vec![],
            ..Default::default()
        }
        .resolve()
    }

    /// A finger landing recognizes nothing yet — long-press needs
    /// the clock to advance and a tap needs the finger to leave.
    /// `None` is what tells the caller to fall back to "redraw on
    /// Started/Moved".
    #[test]
    fn test_drive_touch_event_recognizes_nothing_on_a_bare_start() {
        let mut r = TouchGestureRecognizer::with_thresholds(Duration::from_millis(10), 8.0);
        let kb = keybinds_default();
        let out = drive_touch_event(&mut r, &kb, Phase::Started, 1, (5.0, 6.0), Instant::now());
        assert!(out.is_none());
    }

    /// The long-press path end to end: a finger held past the
    /// threshold is recognized by `tick` (not by `ingest`), reports
    /// the finger's resting position, and resolves through the
    /// keybind table to the default `LongPress` binding.
    #[test]
    fn test_drive_touch_event_resolves_a_long_press_through_the_keybind_table() {
        let mut r = TouchGestureRecognizer::with_thresholds(Duration::from_millis(10), 8.0);
        let kb = keybinds_default();
        let t0 = Instant::now();
        assert!(drive_touch_event(&mut r, &kb, Phase::Started, 1, (5.0, 6.0), t0).is_none());
        // Same finger, same spot, clock advanced past the threshold.
        let late = t0 + Duration::from_millis(50);
        let d = dispatched(drive_touch_event(&mut r, &kb, Phase::Moved, 1, (5.0, 6.0), late));
        assert_eq!(d.gesture_name, MouseGesture::LongPress.key_name());
        assert_eq!(d.cursor_pos, (5.0, 6.0));
        assert_eq!(d.action, Some(Action::EnterResizeMode));
    }

    /// An unbound gesture still yields a dispatch record. This is
    /// the case that must not collapse to `None`: both targets moved
    /// the cursor as soon as a gesture was *recognized*, before ever
    /// consulting the table, so folding "no binding" into "no
    /// gesture" would silently drop that move and leave the next
    /// Action reading a stale cursor.
    #[test]
    fn test_drive_touch_event_reports_the_position_even_when_unbound() {
        let mut r = TouchGestureRecognizer::with_thresholds(Duration::from_millis(10), 8.0);
        let kb = KeybindConfig {
            enter_resize_mode: vec![],
            ..Default::default()
        }
        .resolve();
        let t0 = Instant::now();
        drive_touch_event(&mut r, &kb, Phase::Started, 1, (5.0, 6.0), t0);
        let d = dispatched(drive_touch_event(
            &mut r,
            &kb,
            Phase::Moved,
            1,
            (5.0, 6.0),
            t0 + Duration::from_millis(50),
        ));
        assert_eq!(d.gesture_name, MouseGesture::LongPress.key_name());
        assert_eq!(d.cursor_pos, (5.0, 6.0));
        assert_eq!(d.action, None);
    }

    /// Rebinding the gesture is honored — the same touch now
    /// resolves to a different Action. This is the acceptance
    /// property for the keybind-routed half of the vocabulary: the
    /// table, not the handler, decides.
    #[test]
    fn test_drive_touch_event_honors_a_rebound_gesture() {
        let mut r = TouchGestureRecognizer::with_thresholds(Duration::from_millis(10), 8.0);
        let kb = KeybindConfig {
            enter_resize_mode: vec![],
            select_all: vec!["LongPress".into()],
            ..Default::default()
        }
        .resolve();
        let t0 = Instant::now();
        drive_touch_event(&mut r, &kb, Phase::Started, 1, (1.0, 2.0), t0);
        let d = dispatched(drive_touch_event(
            &mut r,
            &kb,
            Phase::Moved,
            1,
            (1.0, 2.0),
            t0 + Duration::from_millis(50),
        ));
        assert_eq!(d.action, Some(Action::SelectAll));
    }

    /// The lookup passes all-false modifiers: touch devices have no
    /// modifier keys. `action_for_gesture` falls back to the
    /// unmodified binding when no exact-modifier match exists, so a
    /// stray `true` would be invisible unless a *different* Action is
    /// bound to the modified form — which is exactly what this sets
    /// up. The bare binding must win.
    #[test]
    fn test_drive_touch_event_looks_up_with_no_modifiers() {
        let mut r = TouchGestureRecognizer::with_thresholds(Duration::from_millis(10), 8.0);
        let kb = KeybindConfig {
            enter_resize_mode: vec!["LongPress".into()],
            select_all: vec!["Ctrl+LongPress".into()],
            ..Default::default()
        }
        .resolve();
        let t0 = Instant::now();
        drive_touch_event(&mut r, &kb, Phase::Started, 1, (1.0, 2.0), t0);
        let d = dispatched(drive_touch_event(
            &mut r,
            &kb,
            Phase::Moved,
            1,
            (1.0, 2.0),
            t0 + Duration::from_millis(50),
        ));
        assert_eq!(d.action, Some(Action::EnterResizeMode));
    }

    /// A tap routes to the selection effect at the finger's
    /// position, and — the property #35 exists for — it does so with
    /// **every gesture binding cleared**. A tap that resolved through
    /// the table would come back `Dispatch { action: None }` under
    /// this config and select nothing.
    #[test]
    fn test_drive_touch_event_routes_a_tap_to_the_selection_effect() {
        let mut r = TouchGestureRecognizer::with_thresholds(Duration::from_millis(10), 8.0);
        let kb = keybinds_with_no_gesture_bindings();
        let t0 = Instant::now();
        drive_touch_event(&mut r, &kb, Phase::Started, 1, (7.0, 8.0), t0);
        assert_eq!(
            effect_of(drive_touch_event(
                &mut r,
                &kb,
                Phase::Ended,
                1,
                (7.0, 8.0),
                t0 + Duration::from_millis(5)
            )),
            TouchEffect::TapSelect {
                screen_pos: (7.0, 8.0)
            },
        );
    }

    /// A one-finger drag routes to the camera as a pure translation.
    /// `scale` is exactly `1.0`, which is what keeps
    /// `apply_touch_effect` from emitting a `CameraZoom` — and so
    /// from dirtying the connection geometry — for a gesture that
    /// only panned.
    ///
    /// Same cleared-binding config as the tap, for the same reason.
    #[test]
    fn test_drive_touch_event_routes_a_one_finger_drag_to_the_camera() {
        let mut r = TouchGestureRecognizer::with_thresholds(Duration::from_millis(10), 8.0);
        let kb = keybinds_with_no_gesture_bindings();
        let t0 = Instant::now();
        drive_touch_event(&mut r, &kb, Phase::Started, 1, (0.0, 0.0), t0);
        assert_eq!(
            effect_of(drive_touch_event(&mut r, &kb, Phase::Moved, 1, (20.0, 5.0), t0)),
            TouchEffect::CameraStep {
                anchor: (20.0, 5.0),
                pan: (20.0, 5.0),
                scale: 1.0,
            },
        );
    }

    /// Two fingers route to the camera too, carrying both halves of
    /// the transform, and the anchor is the **midpoint** rather than
    /// the moved finger — anchoring the zoom at the raw event
    /// position would slide the canvas out from between the fingers.
    ///
    /// This is what replaced `TwoFingerDrag`: the same physical
    /// gesture, reaching the camera instead of `FastResizeStart` —
    /// which was `NativeOnly`, so on the browser it dispatched,
    /// returned `Unhandled`, and warned.
    #[test]
    fn test_drive_touch_event_routes_a_two_finger_move_to_the_camera() {
        let mut r = TouchGestureRecognizer::with_thresholds(Duration::from_millis(10), 8.0);
        let kb = keybinds_with_no_gesture_bindings();
        let t0 = Instant::now();
        assert!(drive_touch_event(&mut r, &kb, Phase::Started, 1, (0.0, 0.0), t0).is_none());
        assert!(drive_touch_event(&mut r, &kb, Phase::Started, 2, (10.0, 0.0), t0).is_none());
        // Finger 1 to (40, 0): midpoint (5,0) -> (25,0), separation
        // 10 -> 30. Both past the 8px step, and both reported.
        let TouchEffect::CameraStep { anchor, pan, scale } =
            effect_of(drive_touch_event(&mut r, &kb, Phase::Moved, 1, (40.0, 0.0), t0))
        else {
            panic!("two fingers must produce a camera step");
        };
        assert_eq!(anchor, (25.0, 0.0));
        assert_eq!(pan, (20.0, 0.0));
        assert_eq!(scale, 3.0);
    }

    /// The shared last rung scales `EDGE_HIT_TOLERANCE_PX` by the
    /// camera's `canvas_per_pixel`, so the same click hits or misses
    /// the same connection path depending on the zoom — which is the
    /// whole reason the rung takes a camera number rather than a
    /// canvas distance.
    ///
    /// The offset is 50 canvas units off the path, and the two
    /// factors bracket it: `EDGE_HIT_TOLERANCE_PX` is 8, so `0.1`
    /// gives a 0.8-unit budget (miss) and `50.0` gives 400 (hit).
    /// Both are computed from the constant here rather than from the
    /// code under test.
    ///
    /// Fails if the rung stops scaling — passing the raw
    /// `EDGE_HIT_TOLERANCE_PX` makes the zoomed-in case miss too —
    /// or if it stops returning `SelectionState::Edge` for a hit.
    #[test]
    fn test_edge_under_pointer_scales_its_tolerance_by_the_camera() {
        use crate::application::document::tests_common::{load_test_doc, pick_test_edge};
        let doc = load_test_doc();
        let (expected, on_path) = pick_test_edge(&doc);
        let off_path = on_path + Vec2::new(0.0, 50.0);

        // 8 * 0.1 = 0.8 canvas units of slop: far inside 50.
        assert_eq!(
            edge_under_pointer(off_path, &doc.mindmap, CanvasPerPixel::from_ratio(0.1)),
            SelectionState::None,
            "a tight camera must not reach a path 50 units away"
        );
        // 8 * 50 = 400 canvas units: far outside 50.
        assert_eq!(
            edge_under_pointer(off_path, &doc.mindmap, CanvasPerPixel::from_ratio(50.0)),
            SelectionState::Edge(expected),
            "a loose camera must reach the same path"
        );
    }

    /// A pure pan asks the renderer to translate and nothing else.
    ///
    /// Fails if the zoom decree is made unconditional: every
    /// one-finger pan sample would then raise the
    /// connection-geometry dirty flag and buy a scene reprojection
    /// on the next frame, on both targets.
    #[test]
    fn test_camera_step_for_a_pure_pan_asks_for_no_zoom() {
        let (translate, zoom) = camera_step_decrees((10.0, 20.0), (3.0, -4.0), 1.0);
        assert_eq!(translate, RenderDecree::CameraPan(3.0, -4.0));
        assert_eq!(zoom, None);
    }

    /// A scaling step asks for both halves, and anchors the zoom at
    /// the anchor it was handed — the midpoint of the two fingers,
    /// not the finger whose event triggered the step.
    ///
    /// Fails if the anchor is taken from `pan`, or from the origin:
    /// the canvas would then slide out from between the fingers on
    /// every pinch instead of staying pinned under them.
    #[test]
    fn test_camera_step_anchors_the_zoom_where_the_step_says() {
        let (translate, zoom) = camera_step_decrees((10.0, 20.0), (3.0, -4.0), 1.5);
        assert_eq!(translate, RenderDecree::CameraPan(3.0, -4.0));
        assert_eq!(
            zoom,
            Some(RenderDecree::CameraZoom {
                screen_x: 10.0,
                screen_y: 20.0,
                factor: 1.5,
            })
        );
    }

    /// A lifted finger past the tap budget clears the slot and
    /// recognizes nothing. Pinned because `ingest` and `tick` both
    /// run on every call — the `or` must not resurrect a stale
    /// emission from the finger that just left.
    #[test]
    fn test_drive_touch_event_recognizes_nothing_after_a_hold_times_out() {
        let mut r = TouchGestureRecognizer::with_thresholds(Duration::from_millis(10), 8.0);
        let kb = keybinds_default();
        let t0 = Instant::now();
        drive_touch_event(&mut r, &kb, Phase::Started, 1, (5.0, 6.0), t0);
        // 50ms is past the 10ms budget, so this lift is not a tap.
        let late = t0 + Duration::from_millis(50);
        assert!(drive_touch_event(&mut r, &kb, Phase::Ended, 1, (5.0, 6.0), late).is_none());
        // And the now-idle recognizer keeps reporting nothing.
        assert!(drive_touch_event(
            &mut r,
            &kb,
            Phase::Moved,
            1,
            (5.0, 6.0),
            late + Duration::from_millis(50)
        )
        .is_none());
    }
}
