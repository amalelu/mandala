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

use crate::application::document::{EdgeLabelSel, EdgeRef, SectionSel, SelectionState};
use crate::application::keybinds::{Action, ResolvedKeybinds};
use baumhard::mindmap::scene_cache::EdgeKey;

use crate::application::app::input_context_core::InputContextCore;
use crate::application::app::scene_rebuild::{rebuild_after_selection_change, rebuild_all};
use crate::application::app::touch_gesture::{Phase, TouchGestureRecognizer};
use crate::application::app::ClickHit;

use super::apply_create_orphan_node_and_edit;

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
                    core.app_scene,
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
                    rebuild_after_selection_change(
                        &prev,
                        doc,
                        core.interaction_mode,
                        core.mindmap_tree,
                        core.app_scene,
                        core.renderer,
                        core.scene_cache,
                    );
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
        winit::event::TouchPhase::Ended | winit::event::TouchPhase::Cancelled => Phase::Ended,
    }
}

/// A recognized touch gesture, resolved against the keybind table.
///
/// Returned by [`drive_touch_event`]; the caller performs the two
/// effects, because they are the parts the two targets genuinely do
/// differently (native dispatches through `dispatch_action` with the
/// full native context, the browser through `dispatch_compatible`
/// plus a `NativeOnly` warn-log).
pub(in crate::application::app) struct TouchGestureDispatch {
    /// Where the gesture happened. The caller assigns this to
    /// `cursor_pos` **before** dispatching, so the Action reads the
    /// position the finger was at rather than wherever the mouse
    /// cursor last was.
    pub(in crate::application::app) cursor_pos: (f64, f64),
    /// Canonical gesture key name (`"LongPress"`, `"TwoFingerDrag"`,
    /// …) — what the lookup used, and what the browser's warn-log
    /// names.
    pub(in crate::application::app) gesture_name: &'static str,
    /// The bound Action, or `None` when the user has no binding for
    /// this gesture. `None` still yields a `TouchGestureDispatch`
    /// rather than collapsing to `None` at the outer level: both
    /// targets moved the cursor as soon as a gesture was
    /// *recognized*, before ever consulting the table, and folding
    /// the two cases together would silently drop that move.
    pub(in crate::application::app) action: Option<Action>,
}

/// Feed one touch event through the recognizer and resolve whatever
/// it recognizes against the keybind table.
///
/// The whole of the ingest → tick → lookup sequence both runtimes
/// ran as a near-copy. Returns `None` when the event drove no
/// recognition — the caller then falls back to "redraw on
/// Started/Moved" so cursor-following gesture chrome still updates.
///
/// Modifiers are fixed all-false at the lookup: touch devices have
/// no modifier keys, and the `LongPress` / `TwoFingerDrag` bindings
/// don't carry Ctrl/Shift/Alt either.
///
/// Either `ingest` or `tick` can produce at most one recognition per
/// call. When both fire, ingest wins — it is the more recent
/// transition — and the tick's emission is picked up on the next
/// call. Both are always run: `tick` must not be short-circuited by
/// an ingest hit, or the long-press timer would stall.
///
/// The "both fire" case is **not reachable** in the current
/// recognizer, so the `or` ordering is unobservable and no test pins
/// it: `tick` emits only from `OneFinger`, and the two `ingest`
/// emissions come from `TwoFingers` (`TwoFingerDrag`) or return
/// `None` (`OneFinger`'s `Moved`). The order is written down anyway
/// because it is the answer the moment the vocabulary grows a
/// one-finger ingest gesture, and getting it wrong then would
/// silently delay a gesture by one event rather than fail loudly.
pub(in crate::application::app) fn drive_touch_event(
    recognizer: &mut TouchGestureRecognizer,
    keybinds: &ResolvedKeybinds,
    phase: Phase,
    id: u64,
    pos: (f64, f64),
    now: web_time::Instant,
) -> Option<TouchGestureDispatch> {
    let from_ingest = recognizer.ingest(phase, id, pos, now);
    let from_tick = recognizer.tick(now);
    let gesture = from_ingest.or(from_tick)?;
    let gesture_name = gesture.mouse_gesture().key_name();
    Some(TouchGestureDispatch {
        cursor_pos: gesture.pos(),
        gesture_name,
        action: keybinds.action_for_gesture(gesture_name, false, false, false),
    })
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

    /// Every winit phase maps to the recognizer's vocabulary, with
    /// `Cancelled` folding onto `Ended`. `touch_phase` matches
    /// exhaustively, so a new winit variant is a build error rather
    /// than a phase silently handled on one target only.
    #[test]
    fn test_touch_phase_translates_every_winit_phase() {
        use winit::event::TouchPhase as W;
        assert_eq!(touch_phase(W::Started), Phase::Started);
        assert_eq!(touch_phase(W::Moved), Phase::Moved);
        assert_eq!(touch_phase(W::Ended), Phase::Ended);
        assert_eq!(touch_phase(W::Cancelled), Phase::Ended);
    }

    /// A finger landing recognizes nothing yet — long-press needs
    /// the clock to advance. `None` is what tells the caller to fall
    /// back to "redraw on Started/Moved".
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
        let d = drive_touch_event(&mut r, &kb, Phase::Moved, 1, (5.0, 6.0), late)
            .expect("held past the long-press threshold");
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
        let d = drive_touch_event(
            &mut r,
            &kb,
            Phase::Moved,
            1,
            (5.0, 6.0),
            t0 + Duration::from_millis(50),
        )
        .expect("gesture is still recognized when its Action is unbound");
        assert_eq!(d.gesture_name, MouseGesture::LongPress.key_name());
        assert_eq!(d.cursor_pos, (5.0, 6.0));
        assert_eq!(d.action, None);
    }

    /// Rebinding the gesture is honored — the same touch now
    /// resolves to a different Action. This is the acceptance
    /// property for touch: the table, not the handler, decides.
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
        let d = drive_touch_event(
            &mut r,
            &kb,
            Phase::Moved,
            1,
            (1.0, 2.0),
            t0 + Duration::from_millis(50),
        )
        .expect("held past the long-press threshold");
        assert_eq!(d.action, Some(Action::SelectAll));
    }

    /// The other half of the vocabulary, and the half that comes out
    /// of `ingest` rather than `tick` — so this is what proves the
    /// ingest call is not dead. Two fingers down, then one drags the
    /// centroid past the movement threshold.
    ///
    /// It also separates the two positions in play: the *event* is at
    /// (40, 0) while the recognized gesture is the **centroid** at
    /// (25, 0). `cursor_pos` must be the centroid — reporting the raw
    /// event position would put the dispatched Action under the wrong
    /// finger.
    #[test]
    fn test_drive_touch_event_recognizes_a_two_finger_drag_from_ingest() {
        let mut r = TouchGestureRecognizer::with_thresholds(Duration::from_millis(10), 8.0);
        let kb = keybinds_default();
        let t0 = Instant::now();
        assert!(drive_touch_event(&mut r, &kb, Phase::Started, 1, (0.0, 0.0), t0).is_none());
        assert!(drive_touch_event(&mut r, &kb, Phase::Started, 2, (10.0, 0.0), t0).is_none());
        // Centroid moves (5,0) -> (25,0): 20px, past the 8px threshold.
        let d = drive_touch_event(&mut r, &kb, Phase::Moved, 1, (40.0, 0.0), t0)
            .expect("centroid moved past the threshold");
        assert_eq!(d.gesture_name, MouseGesture::TwoFingerDrag.key_name());
        assert_eq!(d.cursor_pos, (25.0, 0.0));
        assert_eq!(d.action, Some(Action::FastResizeStart));
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
        let d = drive_touch_event(
            &mut r,
            &kb,
            Phase::Moved,
            1,
            (1.0, 2.0),
            t0 + Duration::from_millis(50),
        )
        .expect("held past the long-press threshold");
        assert_eq!(d.action, Some(Action::EnterResizeMode));
    }

    /// A lifted finger clears the slot and recognizes nothing. Pinned
    /// because `ingest` returns `None` for `Ended` while `tick` still
    /// runs — the `or` must not resurrect a stale emission.
    #[test]
    fn test_drive_touch_event_recognizes_nothing_after_the_finger_lifts() {
        let mut r = TouchGestureRecognizer::with_thresholds(Duration::from_millis(10), 8.0);
        let kb = keybinds_default();
        let t0 = Instant::now();
        drive_touch_event(&mut r, &kb, Phase::Started, 1, (5.0, 6.0), t0);
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
