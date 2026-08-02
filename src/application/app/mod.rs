// SPDX-License-Identifier: MPL-2.0

//! Application shell: winit event loop, modal state machines,
//! and the dispatch funnel that ties them together. [`Application`]
//! is the binary entry point's root; [`Application::run`]
//! transfers control to the per-target run loop
//! (`run_native` / `run_wasm`) which builds the appropriate
//! `ApplicationHandler` and hands it to winit. Both are plain
//! code-spans, not intra-doc links: each is `cfg`-gated to one
//! target and rustdoc resolves links against the *active*
//! target's module tree, so a link to either breaks the doc
//! build for the other target.
//!
//! **Dispatch funnel.** Every user-driven action — keyboard,
//! mouse-click, console verb, macro replay — flows through
//! `dispatch::dispatch_action` (CODE_CONVENTIONS §3) — a plain
//! code-span for the same reason: it is re-exported from the
//! native-gated `dispatch::native`, so the link does not resolve
//! on `wasm32-unknown-unknown`. Per-event
//! handlers (in `event_keyboard`, `event_mouse_click`,
//! `event_cursor_moved` on native; the per-arm methods of
//! `run_wasm::WasmApp` on WASM) recognize an input gesture,
//! resolve it to an [`crate::application::keybinds::Action`],
//! and call into the funnel. Adding a new behavior is
//! variant + default + arm, in that order; never inline a body
//! in a handler.
//!
//! **Modal state machines.** `text_edit`, `single_line_edit`,
//! `console_input`, and `color_picker_flow` steal keyboard input
//! when open (the §3 carve-out for modals that own the literal
//! `winit::Key` payload). Mouse handlers continue to run; modal
//! commit / cancel routes through `Action::TextEditCommit` /
//! `LabelEditCancel` etc. The two text editors share one steal /
//! release ladder in `modal_editor`.
//!
//! **Cross-platform shape.** Pure logic (gesture recognition,
//! viewport math, hit testing, `Action` resolution) lives in
//! `cfg`-untagged free functions so it compiles for both
//! native and WASM. The native vs. WASM divergence is largely
//! confined to the run-loop entry point; cross-platform
//! `Action` arms route through [`dispatch::action_core`]'s
//! `dispatch_compatible`. See `WASM_CONVERGENCE.md` for the
//! current convergence status.

mod scene_rebuild;
mod text_edit;

// Dispatch funnel — `cross_dispatch` (shared apply_* helpers),
// `action_core` (Compatible-Action dispatcher), `macro_core`
// (cross-platform macro step loop + privilege gate), and `native`
// (native dispatch_action wrapper that adds the NativeOnly arm
// match). The directory's `mod.rs` re-exports the public surface
// so callers stay terse.
pub(crate) mod dispatch;

// Native-only — interactive modal state machines absent on WASM.
// See CLAUDE.md "Dual-target status".
#[cfg(not(target_arch = "wasm32"))]
mod click;
mod click_triggers;
#[cfg(not(target_arch = "wasm32"))]
mod color_picker_flow;
#[cfg(not(target_arch = "wasm32"))]
mod console_input;
#[cfg(not(target_arch = "wasm32"))]
mod drain_frame;
#[cfg(not(target_arch = "wasm32"))]
mod edge_drag;
#[cfg(not(target_arch = "wasm32"))]
mod edge_label_drag;
#[cfg(not(target_arch = "wasm32"))]
mod event_cursor_moved;
#[cfg(not(target_arch = "wasm32"))]
mod event_keyboard;
#[cfg(not(target_arch = "wasm32"))]
mod event_mouse_click;
#[cfg(not(target_arch = "wasm32"))]
mod freeze_watchdog;
#[cfg(not(target_arch = "wasm32"))]
mod input_context;
mod interaction_mode;
// Cross-platform context-bundles for the unified `dispatch_action`
// funnel. Track C from `WASM_CONVERGENCE.md` (final convergence step).
mod input_context_core;
#[cfg(not(target_arch = "wasm32"))]
mod modal_editor;
#[cfg(not(target_arch = "wasm32"))]
mod portal_label_drag;
#[cfg(not(target_arch = "wasm32"))]
mod run_native;
#[cfg(not(target_arch = "wasm32"))]
mod run_native_init;
#[cfg(target_arch = "wasm32")]
mod run_wasm;
#[cfg(not(target_arch = "wasm32"))]
mod single_line_edit;
#[cfg(not(target_arch = "wasm32"))]
mod throttled_interaction;
mod touch_gesture;

// `InputHandlerContext` has 21 fields. Drift surface for new
// fields: the struct in `input_context.rs`, the
// `InitState::input_context()` builder in `run_native.rs`, and
// `dispatch_action`'s signature in `dispatch/native.rs`.
use crate::application::common::{InputMode, WindowMode};

#[cfg(not(target_arch = "wasm32"))]
use crate::application::document::EdgeRef;
#[cfg(not(target_arch = "wasm32"))]
use glam::Vec2;
#[cfg(not(target_arch = "wasm32"))]
#[cfg(not(target_arch = "wasm32"))]
use throttled_interaction::ThrottledDrag;

#[cfg(target_arch = "wasm32")]
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use winit::{event_loop::EventLoop, window::Window};

// `now_ms()` lives in `application::common` — single source for
// the cross-platform monotonic clock both targets use. Re-export
// here so the existing `use super::now_ms` import shape inside
// `app/*` stays put.
pub(crate) use crate::application::common::now_ms;

/// Screen-space click tolerance (in pixels) for edge hit testing. Converted
/// to canvas units via `Renderer::canvas_per_pixel()` so the click target
/// stays visually stable across zoom levels.
#[cfg(not(target_arch = "wasm32"))]
const EDGE_HIT_TOLERANCE_PX: f32 = 8.0;

/// Screen-space click tolerance (in pixels) for grab-handle hit
/// testing — applies uniformly to edge handles and section / node
/// resize handles. With explicit `InteractionMode::Resize` gating
/// handle visibility, the press-time hit-test only competes with
/// itself (no body-vs-handle ambiguity), so 8px is generous for a
/// 14pt ☐ glyph at standard zoom. Touch / accessibility tuning
/// will need this to grow — `KeybindConfig`-side configurability
/// is a future seam.
#[cfg(not(target_arch = "wasm32"))]
const HANDLE_HIT_TOLERANCE_PX: f32 = 8.0;

/// Minimum cursor travel (squared screen-pixel distance) before
/// a `Pending` / `PendingRight` press promotes to its respective
/// drag gesture. The 5px threshold (squared = 25.0) splits "this
/// was a click" from "this is a drag" — small enough that
/// intentional drags engage immediately, large enough that
/// trembling fingers don't accidentally start a drag-to-move
/// from a click. Used by both threshold-cross arms in
/// `event_cursor_moved.rs` (left-button `Pending` and right-
/// button `PendingRight`); single source so a future tweak
/// doesn't drift between the two arms.
#[cfg(not(target_arch = "wasm32"))]
const DRAG_THRESHOLD_SQ_PX: f64 = 25.0;

/// What a single click targeted. Used by [`LastClick`] + the
/// double-click detector so a portal-marker double-click (navigate)
/// is distinguishable from a node double-click (edit text) and from
/// empty-space double-click (create orphan). Two clicks "match" as
/// a double-click only when they have the same `ClickHit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClickHit {
    /// No node and no portal marker under the cursor. Empty-canvas
    /// double-click creates a new orphan unless an edge is selected.
    Empty,
    /// Cursor is inside node `id`'s AABB. `section_idx` is `Some`
    /// when the click resolved to a specific section of a multi-
    /// section node (mirroring `HitTarget::Section`); `None` for
    /// single-section nodes and chrome-only hits. The
    /// `PartialEq`-derived double-click compare honors the
    /// section index, so two slow clicks on different sections
    /// of the same node correctly *don't* count as a double-click,
    /// and a genuine same-section double-click routes the
    /// editor-open path to the targeted section instead of
    /// silently defaulting to section 0.
    Node(String, Option<usize>),
    /// Cursor is inside a portal **icon** marker. `edge` identifies
    /// the owning portal-mode edge; `endpoint` is the node the
    /// hit marker sits above (the double-click pan target is the
    /// *other* endpoint).
    PortalMarker {
        edge: baumhard::mindmap::scene_cache::EdgeKey,
        endpoint: String,
    },
    /// Cursor is inside a portal **text** label — the glyph area
    /// sitting alongside a portal icon. Routes to
    /// `SelectionState::PortalText`, distinct from the icon so
    /// per-channel operations (color / font) target only the
    /// clicked sub-part. Double-click inherits the same
    /// pan-to-partner behavior as `PortalMarker` — the
    /// endpoint identity is shared between icon and text.
    PortalText {
        edge: baumhard::mindmap::scene_cache::EdgeKey,
        endpoint: String,
    },
    /// Cursor is inside a line-mode edge's **label** AABB.
    /// Routes to `SelectionState::EdgeLabel` on single click so
    /// color / font / copy operations target the label instead
    /// of the edge body; double-click opens the inline label
    /// editor, matching the "click to select, dbl to edit"
    /// idiom the `Node` variant already follows.
    EdgeLabel(baumhard::mindmap::scene_cache::EdgeKey),
}

/// Records the previous left-click's time, screen position, and hit
/// target so a second click within a short time + distance window
/// is recognized as a double-click. Double-click fires on the second
/// `Pressed` event, not the second release. `time` is `f64`
/// milliseconds from the cross-platform `now_ms()` helper.
#[derive(Debug, Clone)]
struct LastClick {
    time: f64,
    screen_pos: (f64, f64),
    /// What the first click landed on. Two clicks whose `hit`
    /// values compare equal under `ClickHit`'s derived `PartialEq`
    /// qualify as a double-click.
    hit: ClickHit,
}

/// Double-click window in milliseconds. Matches GNOME/winit convention.
const DOUBLE_CLICK_MS: f64 = 400.0;

/// Double-click maximum distance² in screen-space pixels.
const DOUBLE_CLICK_DIST_SQ: f64 = 16.0 * 16.0;

/// Returns `true` when a new click-down qualifies as a double-click
/// given the previous click. Pure helper so cursor/time math can be
/// unit-tested without a winit event loop.
fn is_double_click(
    prev: &LastClick,
    new_time_ms: f64,
    new_screen_pos: (f64, f64),
    new_hit: &ClickHit,
) -> bool {
    let elapsed = new_time_ms - prev.time;
    if !(0.0..DOUBLE_CLICK_MS).contains(&elapsed) {
        return false;
    }
    let dx = new_screen_pos.0 - prev.screen_pos.0;
    let dy = new_screen_pos.1 - prev.screen_pos.1;
    if dx * dx + dy * dy >= DOUBLE_CLICK_DIST_SQ {
        return false;
    }
    &prev.hit == new_hit
}

/// Scroll delta as a signed line count, for the wheel-gesture
/// lookup.
///
/// winit reports two shapes and they are not interchangeable: a
/// notched wheel reports whole lines, a trackpad or a
/// high-resolution wheel reports pixels. The `/ 50.0` divisor is the
/// pixels-per-line convention this app has always used; it is here
/// rather than at the two call sites so a change to it cannot land
/// on one target only.
///
/// Only the sign is consulted by the gesture lookup today
/// (`> 0.0` → `WheelUp`, otherwise `WheelDown`), but the magnitude is
/// preserved: the console's scrollback accumulates fractional lines
/// through it, and a future per-notch zoom step would want it too.
fn wheel_lines(delta: crate::application::platform::input::MouseScrollDelta) -> f64 {
    use crate::application::platform::input::MouseScrollDelta;
    match delta {
        MouseScrollDelta::LineDelta(_, y) => y as f64,
        MouseScrollDelta::PixelDelta(pos) => pos.y / 50.0,
    }
}

/// The `MouseGesture` a scroll of `lines` names. Split from
/// [`wheel_lines`] because the sign convention — zero and negative
/// both scroll down — is a decision, not arithmetic, and both
/// targets have to make the same one.
fn wheel_gesture(lines: f64) -> crate::application::keybinds::MouseGesture {
    if lines > 0.0 {
        crate::application::keybinds::MouseGesture::WheelUp
    } else {
        crate::application::keybinds::MouseGesture::WheelDown
    }
}

/// Is an inline editor already open on the thing this press landed
/// on?
///
/// When it is, the press must **not** be promoted to a double-click:
/// the double-click would re-open the editor, and re-opening re-seeds
/// the buffer from the committed model value, silently discarding
/// whatever the user had typed. The press instead falls through and
/// the matching release is swallowed as a click-inside.
///
/// Three plain inputs so the guard is pinnable without an event loop
/// (`TEST_CONVENTIONS §T9`):
/// - `edit_node_id` — the node the multi-line text editor has open,
///   `None` when it is closed.
/// - `hit_node` — the node this press landed on, `None` for
///   empty canvas and for non-node hits.
/// - `single_line_match` — whether the single-line editor (edge
///   label / portal caption) is open on *this* press's target. It is
///   passed in rather than resolved here because that editor is
///   native-only today; the browser passes `false`.
///
/// The two editor states are mutually exclusive by construction (the
/// keyboard steal claims whichever opened first), so the `||` is
/// belt-and-braces rather than a case that arises in practice — but
/// a guard that only covered one of them would be a live bug the
/// moment that stops holding.
fn already_editing_same_target(
    edit_node_id: Option<&str>,
    hit_node: Option<&str>,
    single_line_match: bool,
) -> bool {
    edit_node_id.is_some_and(|id| hit_node == Some(id)) || single_line_match
}

/// Warn once per latch that an input resolved to an Action the
/// browser has no body for, so the gesture is a no-op. Returns
/// whether this call is the one that emitted.
///
/// Every input class that consults the keybind table needs this. A
/// user who binds a `NativeOnly` Action to a gesture gets *literally
/// nothing* otherwise — no log, no chrome, no model change — which
/// was rejected as a blocking review finding on the touch path
/// (`Whole-PR review BLK-1`) and is no more acceptable on the
/// double-click or the wheel now that both consult the table too.
/// `warned` is a per-call-site latch so one input class going quiet
/// cannot silence another; `remedy` names what the user can do about
/// it and what unblocks parity.
///
/// A *sanctioned* carve-out — a residual the browser is documented as
/// not finishing, like the edge-label single-line editor — is not
/// this. Those log at `debug!` at their own call site: they are
/// expected, and `warn!` survives into release (`CODE_CONVENTIONS
/// §9`) where a per-double-click warning would be noise. The
/// classification is
/// [`dispatch::classify_unhandled_pointer_dispatch`], not the call
/// site's guesswork.
///
/// **Cross-platform on purpose, though every caller is under
/// `run_wasm/`.** Nothing in the body is browser-specific — an
/// `AtomicBool`, three borrowed values and a `log::warn!` — and while
/// it lived inside the `#![cfg(target_arch = "wasm32")]` module
/// `cargo test` could not reach it, which is what §T9 forbids for
/// platform-shared logic. The `bool` return is what makes the
/// one-shot semantics assertable without a global log sink: this
/// workspace has no log-capture test facility, and installing one
/// would be a process-global mock that §T10 rules out. Callers may
/// ignore it.
pub(super) fn warn_unhandled_native_only_once(
    warned: &std::sync::atomic::AtomicBool,
    gesture: &str,
    action: &crate::application::keybinds::Action,
    remedy: &str,
) -> bool {
    if warned.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    log::warn!(
        "run_wasm: input '{}' is bound to a NativeOnly action ({:?}) with no \
         browser body — the gesture is a no-op. {}",
        gesture,
        action,
        remedy,
    );
    true
}

/// Bag of "what was hit" that the click dispatch on both
/// platforms needs. The collapsed `click_hit` is what
/// double-click detection compares against; the four
/// individual `Option`s are what the editor-state guards
/// (already-editing-same-target) and the WASM
/// `pending_click` snapshot consume — those checks need the
/// underlying hits, not just the collapsed enum.
pub(super) struct ClickHitParts {
    pub(super) click_hit: ClickHit,
    pub(super) hit_node: Option<String>,
    /// Section index inside `hit_node`, when the click landed on a
    /// specific section in a multi-section node. `None` for clicks
    /// on single-section nodes (chrome semantics) and for empty-canvas
    /// clicks.
    pub(super) hit_section_idx: Option<usize>,
    pub(super) portal_text_hit: Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    pub(super) portal_icon_hit: Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    pub(super) edge_label_hit: Option<baumhard::mindmap::scene_cache::EdgeKey>,
}

/// Pure router for "what did this click target?". Runs the
/// node → portal → edge-label priority chain and folds the
/// resolved hits into a single [`ClickHitParts`]. Both the
/// native click handler and the WASM click handler previously
/// open-coded byte-identical versions of this body — they now
/// both call here.
///
/// Priority rationale: node hits beat portal hits (a node
/// under a portal marker is the more common target). Edge-label
/// hits only register when no node / portal sub-part has claimed
/// the click — labels sit along the connection path, and placing
/// them behind the portal check keeps the portal's "floating over
/// a node" behavior correct even if a label happens to overlap.
///
/// The portal rung is a **single** query. Icon and text are
/// sibling leaves of one tree, so
/// [`AppScene::portal_at`](crate::application::scene_host::AppScene::portal_at)
/// resolves both in one BVH descent and names which sub-part won;
/// `portal_text_hit` and `portal_icon_hit` are two views of that
/// one answer and can never both be `Some`. The predecessor ran
/// two independent scans over two hash maps and relied on
/// check-text-first to break a tie the maps could not see.
pub(super) fn compute_click_hit(
    canvas_pos: glam::Vec2,
    mindmap_tree: Option<&mut baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
) -> ClickHitParts {
    use baumhard::mindmap::tree_builder::PortalPart;

    let (hit_node, hit_section_idx) = match mindmap_tree {
        Some(tree) => match crate::application::document::hit_test_target(canvas_pos, tree) {
            Some(crate::application::document::HitTarget::NodeContainer { node_id }) => (Some(node_id), None),
            Some(crate::application::document::HitTarget::Section { node_id, section_idx }) => {
                (Some(node_id), Some(section_idx))
            }
            None => (None, None),
        },
        None => (None, None),
    };

    let portal_hit = if hit_node.is_none() {
        app_scene.portal_at(canvas_pos)
    } else {
        None
    };
    let portal_claimed = portal_hit.is_some();
    let (portal_text_hit, portal_icon_hit) = match portal_hit {
        Some(hit) => {
            let endpoint = (hit.edge_key, hit.endpoint_node_id);
            match hit.part {
                PortalPart::Text => (Some(endpoint), None),
                PortalPart::Icon => (None, Some(endpoint)),
            }
        }
        None => (None, None),
    };
    let edge_label_hit = if hit_node.is_none() && !portal_claimed {
        app_scene.edge_label_at(canvas_pos)
    } else {
        None
    };

    let click_hit = click_hit_from_priority(
        &hit_node,
        hit_section_idx,
        &portal_text_hit,
        &portal_icon_hit,
        &edge_label_hit,
    );

    ClickHitParts {
        click_hit,
        hit_node,
        hit_section_idx,
        portal_text_hit,
        portal_icon_hit,
        edge_label_hit,
    }
}

/// Pure priority-ladder for `ClickHit` construction. Given the
/// four already-resolved hit options, returns the highest-priority
/// `ClickHit` variant that's `Some`. Priority order: node beats
/// portal-text beats portal-icon beats edge-label beats empty.
///
/// Separated from [`compute_click_hit`] so the priority contract
/// can be unit-tested without a `Renderer`. The cascade gating
/// inside `compute_click_hit` already guarantees that at most one
/// of the lower-priority options is `Some` at a time, but this
/// ladder remains correct when callers pass overlapping hits — the
/// ladder is the canonical tie-breaker.
fn click_hit_from_priority(
    hit_node: &Option<String>,
    hit_section_idx: Option<usize>,
    portal_text_hit: &Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    portal_icon_hit: &Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    edge_label_hit: &Option<baumhard::mindmap::scene_cache::EdgeKey>,
) -> ClickHit {
    if let Some(id) = hit_node {
        ClickHit::Node(id.clone(), hit_section_idx)
    } else if let Some((key, ep)) = portal_text_hit {
        ClickHit::PortalText {
            edge: key.clone(),
            endpoint: ep.clone(),
        }
    } else if let Some((key, ep)) = portal_icon_hit {
        ClickHit::PortalMarker {
            edge: key.clone(),
            endpoint: ep.clone(),
        }
    } else if let Some(key) = edge_label_hit {
        ClickHit::EdgeLabel(key.clone())
    } else {
        ClickHit::Empty
    }
}

// Re-export the mode enum, the shared selection→target resolver,
// and the resolver's typed error so the console layer can carry
// `InteractionMode` inside `ConsoleSideEffect` and consumers across
// `application::*` reach a uniform path. Full doc + variant prose
// lives in `interaction_mode.rs`.
pub(in crate::application) use interaction_mode::{
    resolve_resize_target, InteractionMode, ResizeTargetError,
};
// `ResizeTarget` is constructed only inside `interaction_mode.rs`
// (the resolver returns it); production consumers pattern-match
// the result without naming the type. Tests assert on the variant
// shape, so the re-export is `#[cfg(test)]`-gated to avoid an
// unused-import warning on non-test builds. When Batch 4 (fast
// resize) lands a non-test consumer, the gate moves.
#[cfg(test)]
pub(in crate::application) use interaction_mode::ResizeTarget;

/// Everything a left mouse-down captured, held until the cursor
/// either crosses the drag threshold (promoting to a
/// [`ThrottledDrag`]) or comes back up without moving (routing as a
/// click). Boxed inside [`DragState::Pending`] — see there.
#[cfg(not(target_arch = "wasm32"))]
struct PendingPress {
    start_pos: (f64, f64),
    hit_node: Option<String>,
    /// Index inside `hit_node.sections` when the press landed
    /// on a specific section in a multi-section node. `None`
    /// for empty-canvas, single-section, or non-node hits.
    /// Threads through to `handle_click` on the release path
    /// so the post-press selection update can reach for
    /// `SelectionState::Section` when appropriate.
    hit_section_idx: Option<usize>,
    /// If an edge was selected at mouse-down time and the cursor
    /// landed on one of that edge's grab-handles, this records
    /// which handle the user is about to drag. Populated in
    /// `MouseInput::Pressed`, consumed at the threshold-cross
    /// transition in `CursorMoved`. Takes precedence over
    /// `hit_node` — clicking a handle always wins over clicking
    /// the node behind it.
    hit_edge_handle: Option<(EdgeRef, baumhard::mindmap::tree_builder::EdgeHandleKind)>,
    /// If the cursor landed on a portal marker at mouse-down,
    /// this records `(edge_key, endpoint_node_id)` so a drag
    /// past threshold transitions to `Throttled(PortalLabel)`.
    /// Takes precedence over `hit_node` — the marker sits
    /// above a node, but clicking the marker is "grab this
    /// label," not "move this node." Independent of
    /// `hit_edge_handle` because portal-mode edges don't
    /// expose edge-handles in the first place.
    hit_portal_label: Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    /// If the cursor landed on an edge-label AABB at
    /// mouse-down, this records the owning edge key so a
    /// drag past threshold transitions to
    /// `Throttled(EdgeLabel)`. Takes precedence over
    /// `hit_node` — a label hovering over a node behind
    /// it should move as a label, not a node.
    hit_edge_label: Option<baumhard::mindmap::scene_cache::EdgeKey>,
    /// If a section is currently selected and the cursor
    /// landed on one of its 8 resize handles, this records
    /// `(node_id, section_idx, side)` so a drag past
    /// threshold transitions to `Throttled(SectionResize)`.
    /// Takes precedence over `hit_node` — a handle sits on
    /// the section's edge and clicking it is a resize, not
    /// a section drag.
    hit_section_resize_handle: Option<(String, usize, baumhard::mindmap::tree_builder::ResizeHandleSide)>,
    /// If a node is currently `Single`-selected and the
    /// cursor landed on one of its 8 resize handles, this
    /// records `(node_id, side)` so a drag past threshold
    /// transitions to `Throttled(NodeResize)`. Takes
    /// precedence over `hit_node` — clicking a handle on
    /// a selected node is a resize, not a re-selection or
    /// a move-node drag.
    hit_node_resize_handle: Option<(String, baumhard::mindmap::tree_builder::ResizeHandleSide)>,
}

/// Tracks the current drag interaction state.
///
/// Continuous, high-rate-input-driven drag variants
/// (`MovingNode`, `MovingSection`, `SectionResize`, `NodeResize`,
/// `EdgeHandle`, `PortalLabel`, `EdgeLabel`) are collapsed behind
/// the `Throttled` tag. Each carries its pending-state and
/// adaptive throttle as an interaction struct implementing
/// [`throttled_interaction::ThrottledInteraction`]; the per-frame
/// drain in [`run_native::InitState::drain_inputs`] dispatches
/// through [`ThrottledDrag::as_dyn_mut`] without naming the
/// active kind. Adding a new throttled drag is a new variant on
/// `ThrottledDrag` + a struct + a trait impl; nothing about this
/// enum needs to grow.
///
/// `Panning` and `SelectingRect` are *not* throttled — panning is
/// a camera-only decree (no mutation) and rect-select is a
/// lightweight overlay redraw.
#[cfg(not(target_arch = "wasm32"))]
enum DragState {
    /// No drag in progress.
    None,
    /// Mouse is down but hasn't moved past the drag threshold yet.
    ///
    /// Boxed: the press-time hit chain is eight fields wide and made
    /// this the widest `DragState` variant by a factor of six over
    /// its nearest neighbor — `PendingPress` is 384 bytes against
    /// `PendingRight`'s 64 — which every other state, including the
    /// `None` that is live almost all the time, would otherwise pay
    /// for. One allocation per mouse-down.
    ///
    /// With `Pending` and `Throttled` boxed, `PendingRight` is the
    /// widest variant left and `DragState` is 64 bytes (912 before
    /// this pass). `PendingRight` stays unboxed deliberately: at 64
    /// bytes it is not the outlier the other two were, and boxing it
    /// would trade 40 bytes of stack for an allocation on every
    /// right-button press.
    Pending(Box<PendingPress>),
    /// Right-button is down + cursor hasn't moved past the drag
    /// threshold. Press-time hit captures the body of any node /
    /// section under the cursor (no edge-handle / portal-label /
    /// resize-handle precedence — fast-resize is body-only by
    /// design). Threshold-cross promotes to
    /// `Throttled(NodeResize | SectionResize)` via
    /// `Action::FastResizeStart`; release-without-movement fires
    /// the bound `MouseGesture::RightClick` action (default
    /// unbound) and resets to `None`.
    PendingRight {
        start_pos: (f64, f64),
        /// Canvas-space cursor position at press time (already
        /// converted via `Renderer::screen_to_canvas`). Carried
        /// through to `Action::FastResizeStart` so the corner
        /// anchor is computed from where the user *pressed*, not
        /// from where the cursor sat at threshold-cross — plan
        /// §6.3: "Quadrant determined at press time, not
        /// continuously". Without this snapshot the user gets
        /// whichever corner they happened to drag *toward*, which
        /// inverts the gesture's intent on small nodes.
        start_canvas: glam::Vec2,
        /// Press-time hit, body-only. `None` for a press on
        /// empty canvas — release fires `RightClick` regardless
        /// of where the cursor is, but the threshold-cross arm
        /// won't promote to `FastResizeStart` without a node
        /// target.
        hit_node: Option<String>,
        /// Section index inside `hit_node.sections` when the
        /// press landed on a specific section in a multi-section
        /// node. Mirrors the `hit_section_idx` semantics on
        /// `Pending`. `None` for empty-canvas / single-section /
        /// non-node hits.
        hit_section_idx: Option<usize>,
    },
    /// Dragging to pan the camera (started on empty space).
    /// Unthrottled — emits a `CameraPan` decree directly, no
    /// tree or model mutation involved.
    Panning,
    /// Shift+drag on empty space: rubber-band selection rectangle.
    /// Unthrottled — overlay rectangle plus preview highlight is
    /// cheap enough to run every frame.
    SelectingRect {
        /// Canvas-space corner where the drag started.
        start_canvas: Vec2,
        /// Canvas-space corner at current cursor position.
        current_canvas: Vec2,
    },
    /// One of the throttled, mutation-heavy drag gestures —
    /// see [`ThrottledDrag`] for variants. All share the same
    /// adaptive-throttle shell via
    /// [`throttled_interaction::ThrottledInteraction`].
    ///
    /// Boxed: the widest `ThrottledDrag` variant carries a full
    /// pre-drag `MindEdge` snapshot and is more than twice the size
    /// of `Pending`, so an inline payload would make every
    /// `DragState` — including the `None` that is live all but a
    /// few seconds of a session — pay for it. One allocation per
    /// gesture start, at the threshold cross.
    Throttled(Box<ThrottledDrag>),
}

#[cfg(not(target_arch = "wasm32"))]
impl DragState {
    /// Enter a throttled drag. The boxing is an implementation
    /// detail of the variant, not something the nine promotion
    /// sites — seven in `event_cursor_moved`, two in
    /// `dispatch::native` — should each have to spell.
    fn throttled(drag: ThrottledDrag) -> Self {
        Self::Throttled(Box::new(drag))
    }
}

/// Application root — owns the launch options and (on WASM only)
/// the pre-created winit `EventLoop` + canvas `Window`. Constructed
/// from `main.rs` via [`Application::new`]; control transfers to
/// winit on [`Application::run`].
///
/// **WASM variant.** WASM has to attach the canvas to the DOM
/// before the browser's main thread starts dispatching events, so
/// it pre-creates the window and the event loop in
/// [`Application::new`] and hands them to `run_wasm::run` together.
/// The `#[allow(deprecated)]` on the WASM constructor's
/// `event_loop.create_window(...)` call records this asymmetry —
/// ditto the `event_loop` and `window` fields, which only exist on
/// the WASM side.
#[cfg(target_arch = "wasm32")]
pub struct Application {
    options: Options,
    event_loop: EventLoop<()>,
    window: Arc<Window>,
}

/// Application root — owns the launch options. Constructed from
/// `main.rs` via [`Application::new`]; control transfers to winit
/// on [`Application::run`].
///
/// **Native variant.** Native creates the window inside winit's
/// `ApplicationHandler::resumed` callback (the modern winit 0.30
/// path), so the struct here only carries [`Options`]. The window
/// itself lives on the run-loop's `InitState`, materialized lazily
/// on first resume.
#[cfg(not(target_arch = "wasm32"))]
pub struct Application {
    options: Options,
}

impl Application {
    #[cfg(target_arch = "wasm32")]
    pub fn new(options: Options) -> Self {
        let event_loop = EventLoop::new().expect("Could not create an EventLoop");

        // Pre-creating the window here on winit 0.30 is deprecated in
        // favor of `ActiveEventLoop::create_window` inside
        // `ApplicationHandler::resumed`. The native path takes that
        // route; the WASM path still pre-creates because
        // `run_wasm::run` attaches the canvas and installs DOM event
        // listeners before the event loop starts.
        #[allow(deprecated)]
        let window = event_loop
            .create_window(Window::default_attributes())
            .expect("Failed to create application window");

        Application {
            options,
            event_loop,
            window: Arc::new(window),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(options: Options) -> Self {
        Application { options }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn run(self) {
        run_native::run(self)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn run(self) {
        run_wasm::run(self)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn into_options(self) -> Options {
        self.options
    }
}

/// Launch options assembled by `main.rs` from CLI flags + env
/// detection, frozen into [`Application`] at startup. Read once
/// per launch; never mutated post-construction.
#[derive(Clone)]
pub struct Options {
    /// Hint to wgpu's adapter selection: prefer integrated /
    /// low-power GPUs over discrete ones. Useful on laptops
    /// where the discrete GPU would burn battery for a render
    /// load Mandala can run on the iGPU.
    pub launch_gpu_prefer_low_power: bool,
    /// `true` to short-circuit the event loop after the first
    /// frame — used by smoke-tests / CI captures that just need
    /// to verify a single render pass succeeds. The interactive
    /// run never sets this.
    pub should_exit: bool,
    /// Window startup mode (windowed / fullscreen / maximized);
    /// see [`WindowMode`].
    pub window_mode: WindowMode,
    /// User-config UI-scale offset. The renderer scales every
    /// glyph by `1.0 + ui_scale * UI_SCALE_STEP`; `0` is the
    /// neutral default. Negative shrinks, positive grows.
    pub ui_scale: i8,
    /// Static title bar text. `&'static` because it's set at
    /// compile time and never user-edited.
    pub window_title_text: &'static str,
    /// Input dispatch mode (direct vs. instruction-mapped).
    /// See [`InputMode`].
    pub input_mode: InputMode,
    /// CPU core count detected at startup. Reserved for future
    /// thread-pool sizing; today the app is single-threaded so
    /// this is informational only.
    pub avail_cores: usize,
    /// `true` when wgpu requires the renderer to live on the
    /// main thread (the macOS / wasm constraint). Set by
    /// platform detection in `main.rs`.
    pub render_must_be_main: bool,
    /// Path to the `.mindmap.json` file to load at startup.
    /// Native: filesystem path; WASM: a fetch-relative URL
    /// resolved against the page origin.
    pub mindmap_path: String,
    /// The user's keybinding configuration (already loaded from file or
    /// defaults). The event loop resolves this into a `ResolvedKeybinds`
    /// at startup and dispatches keyboard events through it.
    pub keybind_config: crate::application::keybinds::KeybindConfig,
}

// Unit tests for pure helpers (cursor math, caret insertion,
// double-click detection, Baumhard mutation round-trip). Event-loop
// integration is verified manually via `cargo run`.

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests;
