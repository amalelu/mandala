// SPDX-License-Identifier: MPL-2.0

//! Document-lifecycle apply_* helpers — undo, create / orphan /
//! delete / edit on the current selection, and the cross-platform
//! clipboard arms (copy / cut / paste). Each routes through the
//! shared rebuild plumbing so geometry-changing edits trigger a
//! full scene rebuild while read-only ones (copy) skip it.

use crate::application::app::interaction_mode;
use crate::application::document::MindMapDocument;

use super::RebuildContext;

/// Joiner between multi-target clipboard fragments. Doubled `\n`
/// so the paste path can split unambiguously even when an
/// individual section's text contains `\n` line breaks from the
/// inline editor.
const MULTI_TARGET_SEPARATOR: &str = "\n\n";

/// Resolve a custom-mutation key binding and apply it through the same
/// path the click-trigger handler in `click.rs` uses: animation-aware
/// (`start_animation` when `timing.duration_ms > 0`), and always
/// invoking `apply_document_actions`. Returns `true` when a mutation
/// was found and applied.
///
/// This is the third and lowest tier of the keyboard resolution chain
/// (Action → Macro → CustomMutation, CONCEPTS §5). It takes
/// [`InputContextCore`](crate::application::app::input_context_core::InputContextCore)
/// rather than the native context because both
/// targets' keyboard handlers run the same chain: a
/// `custom_mutation_bindings` entry that fires on the desktop has to
/// fire in the browser too, and before this lift the browser's chain
/// stopped one tier short.
///
/// Rebuilds via bare `rebuild_all`, deliberately **not**
/// [`RebuildContext::rebuild_after_geometry_change`]:
/// [`super::apply_keybind_custom_mutation`] already clears the
/// connection cache on the non-animated branch, and the animated
/// branch leaves the cache for the animation envelope to invalidate —
/// an extra clear here would cost a full re-sample on every animated
/// keystroke.
pub(in crate::application::app) fn dispatch_custom_mutation_for_key(
    core: &mut super::super::super::input_context_core::InputContextCore<'_>,
    key_name: &str,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> bool {
    let id = match core.keybinds.custom_mutation_for(key_name, ctrl, shift, alt) {
        Some(s) => s.to_string(),
        None => return false,
    };
    let Some(doc) = core.document.as_deref_mut() else {
        return false;
    };
    let crate::application::document::SelectionState::Single(nid) = doc.selection.clone() else {
        return false;
    };
    let Some(cm) = doc.mutation_registry.get(&id).cloned() else {
        return false;
    };
    let now = crate::application::app::now_ms() as u64;
    let applied =
        super::apply_keybind_custom_mutation(doc, core.mindmap_tree, core.scene_cache, &cm, &nid, now);
    if applied {
        crate::application::app::scene_rebuild::rebuild_all(
            doc,
            core.interaction_mode,
            core.mindmap_tree,
            core.app_scene,
            core.renderer,
            core.scene_cache,
        );
    }
    applied
}

/// Walk the undo stack one step back. If an animation is in flight
/// when undo fires, fast-forward it first so the undo lands on a
/// settled scene state rather than mid-transition (otherwise the
/// undo'd write competes with the still-running animation envelope).
/// Both dispatchers route through this so the fast-forward
/// behaviour is platform-uniform — pre-Track-A WASM skipped it.
pub(in crate::application::app) fn apply_undo(rc: &mut RebuildContext<'_>) {
    if rc.document.has_active_animations() {
        rc.document.fast_forward_animations(rc.mindmap_tree.as_mut());
    }
    if rc.document.undo() {
        rc.rebuild_after_geometry_change();
    }
}

/// Create a new orphan node at the given canvas-space position
/// and select it. Triggers a geometry-change rebuild because the
/// new node may shift connection routes / introduce new edges.
pub(in crate::application::app) fn apply_create_orphan_node(
    canvas_pos: glam::Vec2,
    rc: &mut RebuildContext<'_>,
) {
    rc.document.create_orphan_and_select(canvas_pos);
    rc.rebuild_after_geometry_change();
}

/// Create a new orphan node at `canvas_pos`, select it, rebuild,
/// then open the inline text editor on the new node pre-cleared.
///
/// The single body for `Action::CreateOrphanNodeAndEdit` — this
/// existed in three copies before #29 (here, a private helper in
/// `dispatch/native.rs`, and inline in the browser's double-click
/// ladder). All three callers now land here:
///
/// - the keyboard shape, via `dispatch_compatible`'s
///   `CreateOrphanNodeAndEdit` arm, which supplies `cursor_pos`;
/// - both targets' mouse-driven empty-canvas double-click, via
///   [`super::DoubleClickRoute::CreateOrphanAndEdit`], which
///   supplies `DispatchHit::canvas_pos`.
///
/// The position is the only thing that ever differed between them.
pub(in crate::application::app) fn apply_create_orphan_node_and_edit(
    canvas_pos: glam::Vec2,
    rc: &mut RebuildContext<'_>,
    text_edit_state: &mut super::super::super::text_edit::TextEditState,
) {
    let new_id = rc.document.create_orphan_and_select(canvas_pos);
    rc.rebuild_after_geometry_change();
    super::super::super::text_edit::open_text_edit(
        &new_id,
        true,
        rc.document,
        text_edit_state,
        rc.mindmap_tree,
        rc.app_scene,
        rc.renderer,
    );
}

/// Detach every currently-selected node from its parent. No-op
/// when nothing is selected or every selected node was already a
/// root.
pub(in crate::application::app) fn apply_orphan_selection(rc: &mut RebuildContext<'_>) {
    if rc.document.apply_orphan_selection_with_undo() {
        rc.rebuild_after_geometry_change();
    }
}

/// Delete the current selection. Pre-flight checks (selection
/// non-empty, deletable) live in the document method; this helper
/// just gates the rebuild.
pub(in crate::application::app) fn apply_delete_selection(rc: &mut RebuildContext<'_>) {
    if rc.document.apply_delete_selection() {
        rc.rebuild_after_geometry_change();
    }
}

/// What `apply_enter_node_edit` should do given the current
/// (selection, model) state. Pure function output — the testable
/// half of the action helper, separated from the renderer-driving
/// side of `apply_enter_node_edit` itself (per `TEST_CONVENTIONS
/// §T8`, the renderer-touching arms can't run under `cargo test`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::application::app) enum EnterNodeEditPlan {
    /// Selection has no node target (Multi / MultiSection / Edge /
    /// None). Caller logs a warning and bails.
    NoTarget,
    /// Single-section short-circuit: flip mode to `NodeEdit` AND
    /// open the section editor on section 0 in the same call.
    /// Preserves the legacy "Enter on a node opens the editor" UX
    /// for migrated maps.
    SingleSectionShortCircuit { node_id: String },
    /// Multi-section: flip mode to `NodeEdit { node_id }` only;
    /// the user picks a specific section before entering the
    /// editor (a second Enter, or a click on a section).
    EnterMultiSection { node_id: String },
}

/// Resolve the [`EnterNodeEditPlan`] from selection + model state
/// without any renderer / tree side effects. Pulls the testable
/// decision logic out of `apply_enter_node_edit` so unit tests can
/// pin every branch (TEST_CONVENTIONS §T8 — no GPU dependency).
pub(in crate::application::app) fn resolve_enter_node_edit_plan(
    selection: &crate::application::document::SelectionState,
    mindmap: &baumhard::mindmap::model::MindMap,
) -> EnterNodeEditPlan {
    let Some(node_id) = selection.primary_node_id().map(str::to_string) else {
        return EnterNodeEditPlan::NoTarget;
    };
    let section_count = mindmap
        .nodes
        .get(&node_id)
        .map(|n| n.sections.len())
        .unwrap_or(0);
    if section_count <= 1 {
        EnterNodeEditPlan::SingleSectionShortCircuit { node_id }
    } else {
        EnterNodeEditPlan::EnterMultiSection { node_id }
    }
}

/// Enter NodeEdit mode on the currently-selected node. Resolves the
/// owning node via `selection.primary_node_id()` (Single / Section /
/// SectionRange). Multi / MultiSection / edge / None warn and bail.
///
/// **Single-section short-circuit**: when the active node has
/// `sections.len() <= 1`, opens the text editor on section 0 in the
/// same call. This preserves today's "Enter on a node opens the
/// editor" UX for legacy migrated maps. Multi-section nodes stop
/// at NodeEdit mode — the user picks a section (click or
/// `section edit <idx>` console verb) and presses Enter again to
/// enter SectionEdit.
///
/// `clean: true` opens the editor with an empty buffer (mirrors
/// `EditSelectionClean`'s posture) — only used in the
/// single-section short-circuit path.
pub(in crate::application::app) fn apply_enter_node_edit(
    clean: bool,
    rc: &mut RebuildContext<'_>,
    text_edit_state: &mut super::super::super::text_edit::TextEditState,
) -> bool {
    use super::super::super::interaction_mode::InteractionMode;

    let plan = resolve_enter_node_edit_plan(&rc.document.selection, &rc.document.mindmap);
    match plan {
        EnterNodeEditPlan::NoTarget => {
            log::warn!(
                "EnterNodeEdit: selection has no primary node \
                 (Multi / MultiSection / Edge / None) — nothing to edit"
            );
            false
        }
        EnterNodeEditPlan::SingleSectionShortCircuit { node_id } => {
            *rc.interaction_mode = InteractionMode::NodeEdit { node_id: node_id.clone() };
            // Short-circuit path: this call ALSO flipped mode to
            // NodeEdit, so on close the editor must revert mode to
            // Default — a single-section node has nothing else to
            // edit, so leaving the user in NodeEdit + dimming is a
            // UX dead-end.
            super::super::super::text_edit::open_text_edit_with_close_target(
                &node_id,
                clean,
                true, // exit_to_default_on_close
                rc.document,
                text_edit_state,
                rc.mindmap_tree,
                rc.app_scene,
                rc.renderer,
            );
            true
        }
        EnterNodeEditPlan::EnterMultiSection { node_id } => {
            *rc.interaction_mode = InteractionMode::NodeEdit { node_id };
            // Multisection: stop at NodeEdit so the user can pick
            // a section. Rebuild so the dimming + status-bar
            // visuals catch the mode change.
            rc.rebuild_after_selection_change();
            true
        }
    }
}

/// What `apply_enter_section_edit` should do given the current
/// (mode, selection) state. Pure function output — testable half
/// of the action helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::application::app) enum EnterSectionEditPlan {
    /// Mode wasn't NodeEdit. Caller logs a warning and bails.
    NotInNodeEdit,
    /// Mode was NodeEdit but the selection's owner mismatched the
    /// active NodeEdit target (e.g. user clicked a sibling node).
    /// Caller logs a warning naming both ids and bails.
    OwnerMismatch { active_node: String, owner: Option<String> },
    /// Open the section editor on `active_node`. The caller takes
    /// the renderer-driving side from here.
    OpenEditor { active_node: String },
}

/// Resolve the [`EnterSectionEditPlan`] from (mode, selection)
/// without renderer / tree side effects. Lifts the testable
/// preconditions out of `apply_enter_section_edit`.
pub(in crate::application::app) fn resolve_enter_section_edit_plan(
    interaction_mode: &super::super::super::interaction_mode::InteractionMode,
    selection: &crate::application::document::SelectionState,
) -> EnterSectionEditPlan {
    use super::super::super::interaction_mode::InteractionMode;
    let active_node = match interaction_mode {
        InteractionMode::NodeEdit { node_id } => node_id.clone(),
        _ => return EnterSectionEditPlan::NotInNodeEdit,
    };
    let owner = selection.primary_node_id().map(str::to_string);
    if owner.as_deref() != Some(&active_node) {
        return EnterSectionEditPlan::OwnerMismatch {
            active_node,
            owner,
        };
    }
    EnterSectionEditPlan::OpenEditor { active_node }
}

/// Open the section text editor on the active section while in
/// NodeEdit mode. Preconditions:
/// - `interaction_mode == InteractionMode::NodeEdit { node_id }`.
/// - Selection picks the section: `Section(s)` / `SectionRange { sel: s, .. }`
///   use `s.section_idx`; `Single(node_id)` defaults to section 0.
///
/// `clean: true` opens the editor with an empty buffer.
///
/// Returns `true` if the editor opened. Mode stays at `NodeEdit`
/// (closing the editor returns to `NodeEdit`, not `Default`).
pub(in crate::application::app) fn apply_enter_section_edit(
    clean: bool,
    rc: &mut RebuildContext<'_>,
    text_edit_state: &mut super::super::super::text_edit::TextEditState,
) -> bool {
    let plan = resolve_enter_section_edit_plan(&*rc.interaction_mode, &rc.document.selection);
    match plan {
        EnterSectionEditPlan::NotInNodeEdit => {
            log::warn!("EnterSectionEdit: no active NodeEdit mode; nothing to edit");
            false
        }
        EnterSectionEditPlan::OwnerMismatch { active_node, owner } => {
            log::warn!(
                "EnterSectionEdit: selection owner ≠ active NodeEdit node ({:?} vs {})",
                owner, active_node
            );
            false
        }
        EnterSectionEditPlan::OpenEditor { active_node } => {
            super::super::super::text_edit::open_text_edit(
                &active_node,
                clean,
                rc.document,
                text_edit_state,
                rc.mindmap_tree,
                rc.app_scene,
                rc.renderer,
            );
            true
        }
    }
}

/// Resolve the current `SelectionState` into a `ResizeTarget` and
/// flip the active interaction mode to `Resize { target }`. On a
/// non-resizable selection logs the resolution failure and leaves
/// mode untouched. Cross-platform: touches mode + model + scene
/// rebuild only.
///
/// Resolution logic is shared with the `mode resize` console verb
/// via [`interaction_mode::resolve_resize_target`].
pub(in crate::application::app) fn apply_enter_resize_mode(rc: &mut RebuildContext<'_>) {
    use interaction_mode::{
        resolve_resize_target, InteractionMode, ResizeTargetError,
    };

    match resolve_resize_target(&rc.document.selection, &rc.document.mindmap) {
        Ok(target) => {
            *rc.interaction_mode = InteractionMode::Resize { target };
            rc.rebuild_after_selection_change();
        }
        Err(ResizeTargetError::NoSelection) => {
            log::warn!("EnterResizeMode: no selection; nothing to resize");
        }
        Err(ResizeTargetError::MultiTarget) => {
            log::warn!(
                "EnterResizeMode: multi-target selection — single-target only; \
                 select a single node or section first"
            );
        }
        Err(ResizeTargetError::SectionFillParent { node_id, section_idx }) => {
            log::warn!(
                "EnterResizeMode: section {}[{}] is fill-parent (no Some size); cannot resize",
                node_id, section_idx,
            );
        }
        Err(ResizeTargetError::EdgeOrPortal) => {
            log::warn!("EnterResizeMode: edge / label / portal selection — not resizable");
        }
    }
}

// ── Clipboard ───────────────────────────────────────────────────
//
// Cross-platform: `clipboard::{read,write}_clipboard` are logged
// stubs on WASM (pending the async-clipboard integration). The
// trait-driven walk over `selection_targets` is identical on both
// targets — `console::traits` compiles WASM-side because it only
// reaches into the document model, not the cfg-gated console
// runtime.

/// The section-clipboard payload produced by a single-section copy:
/// the plain text (mirrored to the OS clipboard) plus the structured
/// `SectionPayload` (used for within-app section→section paste to
/// round-trip per-run formatting and section geometry).
pub(in crate::application::app) struct StructuredSection {
    pub text: String,
    pub payload: crate::application::document::SectionPayload,
}

pub(in crate::application::app) struct ComputedCopy {
    pub joined_text: String,
    /// `Some(s)` only for single-section copies that produced a structured
    /// payload; `None` for plain-text targets and for multi-section copies
    /// (no payload variant exists today).
    pub structured: Option<StructuredSection>,
}

/// Iterate the current selection's clipboard-eligible targets and
/// build the clipboard payload, without touching the OS clipboard.
/// Clears the thread-local structured buffer up-front (a stale
/// single-section payload from a prior copy would otherwise win the
/// byte-equal probe on the next paste). The clear lives here — not in
/// `apply_copy_or_cut` — so unit tests can exercise the full
/// copy/clear contract without needing an OS clipboard.
/// Returns `None` if no target accepted; `Some(ComputedCopy)` otherwise.
///
/// Cut path mutates `doc` (clears source text per `clipboard_cut`).
pub(in crate::application::app) fn prepare_copy_or_cut(
    is_cut: bool,
    doc: &mut MindMapDocument,
) -> Option<ComputedCopy> {
    use crate::application::console::traits::{
        selection_targets, view_for, ClipboardContent, HandlesCopy, HandlesCut,
    };
    crate::application::clipboard::clear_section_clipboard();

    let targets = selection_targets(&doc.selection);
    let mut text_payloads: Vec<String> = Vec::new();
    let mut section_texts: Vec<String> = Vec::new();
    let mut first_section_payload: Option<crate::application::document::SectionPayload> = None;
    for tid in &targets {
        let mut view = view_for(doc, tid);
        let content = if is_cut { view.clipboard_cut() } else { view.clipboard_copy() };
        match content {
            ClipboardContent::Text(text) => {
                text_payloads.push(text);
            }
            ClipboardContent::Section { text, payload } => {
                if section_texts.is_empty() {
                    first_section_payload = Some(payload);
                }
                section_texts.push(text);
            }
            ClipboardContent::Empty | ClipboardContent::NotApplicable => {}
        }
    }
    if !text_payloads.is_empty() {
        return Some(ComputedCopy {
            joined_text: text_payloads.join(MULTI_TARGET_SEPARATOR),
            structured: None,
        });
    }
    if !section_texts.is_empty() {
        let joined_text = section_texts.join(MULTI_TARGET_SEPARATOR);
        let structured = if section_texts.len() == 1 {
            first_section_payload.map(|payload| StructuredSection {
                text: section_texts.into_iter().next().expect("len == 1"),
                payload,
            })
        } else {
            None
        };
        return Some(ComputedCopy { joined_text, structured });
    }
    None
}

/// Copy or Cut the current selection's clipboard-eligible content to
/// the system clipboard. Cut additionally clears the source component's
/// text (via `prepare_copy_or_cut` → `clipboard_cut`). Returns `false`
/// when no target accepted the operation. Clearing + iteration are
/// handled by `prepare_copy_or_cut`; this wrapper commits the OS
/// clipboard and structured-buffer side effects.
pub(in crate::application::app) fn apply_copy_or_cut(is_cut: bool, doc: &mut MindMapDocument) -> bool {
    let Some(c) = prepare_copy_or_cut(is_cut, doc) else { return false };
    crate::application::clipboard::write_clipboard(&c.joined_text);
    if let Some(s) = c.structured {
        crate::application::clipboard::write_section_clipboard(s.text, s.payload);
    }
    true
}

/// Read the system clipboard and paste into every clipboard-eligible
/// target in the current selection. Triggers a geometry rebuild iff
/// at least one target accepted the paste.
///
/// For multi-target selections, the OS clipboard is split on
/// `MULTI_TARGET_SEPARATOR` and zipped 1:1 when fragment count
/// equals target count (round-trip from a Mandala multi-target
/// copy); otherwise the full blob broadcasts to every target
/// (cross-app paste, count mismatch).
///
/// **Broadcast structured-buffer guard.** When the multi-target
/// path falls back to broadcast, the in-process `SECTION_BUFFER`
/// is cleared up-front. Otherwise a structured payload from a
/// prior single-section copy would survive the byte-equal probe
/// inside each per-target `clipboard_paste` call and silently
/// apply the same `SectionPayload` (offset, size, channel,
/// trigger_bindings) to every target — distinct sections of the
/// same node would end up with identical geometry, corrupting
/// the model.
pub(in crate::application::app) fn apply_paste(rc: &mut RebuildContext<'_>) {
    use crate::application::console::traits::{selection_targets, view_for, HandlesPaste, Outcome};
    let Some(text) = crate::application::clipboard::read_clipboard() else {
        return;
    };
    let targets = selection_targets(&rc.document.selection);
    let fragments = split_paste_for_targets(&text, targets.len());
    if is_broadcast_paste(fragments.is_none(), targets.len()) {
        crate::application::clipboard::clear_section_clipboard();
    }
    let mut any_applied = false;
    for (i, tid) in targets.iter().enumerate() {
        let mut view = view_for(rc.document, tid);
        let to_paste: &str = match &fragments {
            Some(frags) => frags[i],
            None => &text,
        };
        if let Outcome::Applied = view.clipboard_paste(to_paste) {
            any_applied = true;
        }
    }
    if any_applied {
        rc.rebuild_after_geometry_change();
    }
}

/// Decide whether the OS clipboard text should be split 1:1 with
/// the paste targets (round-trip from a Mandala multi-target copy)
/// or broadcast verbatim to every target (cross-app paste / count
/// mismatch). Returns `Some(fragments)` when split-and-zip applies,
/// `None` when the caller should broadcast `text` to every target.
///
/// The split is gated on `target_count > 1` AND
/// `fragments.len() == target_count`. A single `\n\n` in a
/// pasted-from-another-app blob with a 2-target selection that
/// happens to match by coincidence will round-trip; that's
/// indistinguishable from a Mandala copy by content alone, so the
/// behavior matches what the user typed on the source side.
/// True when a paste must clear the structured section buffer:
/// fragments couldn't be split per-target AND there's more than
/// one target. Without the clear, a stale single-section payload
/// would broadcast its non-text fields to every target.
fn is_broadcast_paste(fragments_is_none: bool, target_count: usize) -> bool {
    fragments_is_none && target_count > 1
}

fn split_paste_for_targets(text: &str, target_count: usize) -> Option<Vec<&str>> {
    if target_count <= 1 {
        return None;
    }
    let fragments: Vec<&str> = text.split(MULTI_TARGET_SEPARATOR).collect();
    if fragments.len() == target_count {
        Some(fragments)
    } else {
        None
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::{
        apply_copy_or_cut, prepare_copy_or_cut, resolve_enter_node_edit_plan,
        resolve_enter_section_edit_plan, split_paste_for_targets, EnterNodeEditPlan,
        EnterSectionEditPlan, MULTI_TARGET_SEPARATOR,
    };
    use crate::application::app::interaction_mode::InteractionMode;
    use crate::application::clipboard::{
        clear_section_clipboard, read_section_clipboard, write_section_clipboard,
    };
    use crate::application::document::tests_common::{load_test_doc, pinned_two_section_node};
    use crate::application::document::{
        EdgeRef, SectionPayload, SectionSel, SelectionState,
    };

    // ── EnterNodeEdit plan resolution ────────────────────────────

    /// `Single(node_id)` selection on a muli-section node →
    /// EnterMultiSection (mode flip only, no editor open).
    #[test]
    fn test_resolve_enter_node_edit_single_on_multi_section_enters_multi() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id.clone());
        let plan = resolve_enter_node_edit_plan(&doc.selection, &doc.mindmap);
        assert_eq!(plan, EnterNodeEditPlan::EnterMultiSection { node_id: id });
    }

    /// `Single` selection on a single-section node →
    /// SingleSectionShortCircuit (mode + editor open in one call).
    #[test]
    fn test_resolve_enter_node_edit_single_on_single_section_short_circuits() {
        let (mut doc, id) = pinned_two_section_node();
        doc.mindmap.nodes.get_mut(&id).unwrap().sections.truncate(1);
        doc.selection = SelectionState::Single(id.clone());
        let plan = resolve_enter_node_edit_plan(&doc.selection, &doc.mindmap);
        assert_eq!(plan, EnterNodeEditPlan::SingleSectionShortCircuit { node_id: id });
    }

    /// `Section` selection routes to its owning node.
    #[test]
    fn test_resolve_enter_node_edit_section_routes_to_owner_node() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel::new(&id, 1));
        let plan = resolve_enter_node_edit_plan(&doc.selection, &doc.mindmap);
        assert_eq!(plan, EnterNodeEditPlan::EnterMultiSection { node_id: id });
    }

    /// `SectionRange` selection (the post-shift-select close shape)
    /// routes to its owning node — Plan §4.5 / `primary_node_id`
    /// shape-equivalence.
    #[test]
    fn test_resolve_enter_node_edit_section_range_routes_to_owner_node() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::SectionRange {
            sel: SectionSel::new(&id, 1),
            range: (0, 3),
        };
        let plan = resolve_enter_node_edit_plan(&doc.selection, &doc.mindmap);
        assert_eq!(plan, EnterNodeEditPlan::EnterMultiSection { node_id: id });
    }

    /// `None` selection → NoTarget (caller logs and bails).
    #[test]
    fn test_resolve_enter_node_edit_none_selection_returns_no_target() {
        let mut doc = load_test_doc();
        doc.selection = SelectionState::None;
        let plan = resolve_enter_node_edit_plan(&doc.selection, &doc.mindmap);
        assert_eq!(plan, EnterNodeEditPlan::NoTarget);
    }

    /// `Multi` selection → NoTarget. NodeEdit is single-target by
    /// design.
    #[test]
    fn test_resolve_enter_node_edit_multi_selection_returns_no_target() {
        let mut doc = load_test_doc();
        let ids: Vec<String> = doc.mindmap.nodes.keys().take(2).cloned().collect();
        doc.selection = SelectionState::Multi(ids);
        let plan = resolve_enter_node_edit_plan(&doc.selection, &doc.mindmap);
        assert_eq!(plan, EnterNodeEditPlan::NoTarget);
    }

    /// `Edge` selection → NoTarget — edges aren't node-scoped.
    #[test]
    fn test_resolve_enter_node_edit_edge_selection_returns_no_target() {
        let mut doc = load_test_doc();
        let er = doc
            .mindmap
            .edges
            .first()
            .map(|e| EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type))
            .expect("test doc has edges");
        doc.selection = SelectionState::Edge(er);
        let plan = resolve_enter_node_edit_plan(&doc.selection, &doc.mindmap);
        assert_eq!(plan, EnterNodeEditPlan::NoTarget);
    }

    /// Stale selection pointing at a node that doesn't exist falls
    /// to the SingleSection branch (section_count = 0). The caller
    /// path then opens an editor on a missing node, which `open_text_edit`
    /// handles gracefully (returns early). Pin so a future change
    /// to the resolver makes this branch explicit rather than silent.
    #[test]
    fn test_resolve_enter_node_edit_stale_node_id_falls_to_short_circuit() {
        let mut doc = load_test_doc();
        doc.selection = SelectionState::Single("nonexistent-node".to_string());
        let plan = resolve_enter_node_edit_plan(&doc.selection, &doc.mindmap);
        assert_eq!(
            plan,
            EnterNodeEditPlan::SingleSectionShortCircuit {
                node_id: "nonexistent-node".to_string()
            }
        );
    }

    // ── EnterSectionEdit plan resolution ─────────────────────────

    /// Default mode → NotInNodeEdit. SectionEdit is only meaningful
    /// inside NodeEdit (it's the modal scope under which per-section
    /// editing makes sense).
    #[test]
    fn test_resolve_enter_section_edit_default_mode_returns_not_in_node_edit() {
        let mut doc = load_test_doc();
        let id = doc.mindmap.nodes.keys().next().cloned().expect("nodes");
        doc.selection = SelectionState::Single(id);
        let plan = resolve_enter_section_edit_plan(&InteractionMode::Default, &doc.selection);
        assert_eq!(plan, EnterSectionEditPlan::NotInNodeEdit);
    }

    /// Resize mode → NotInNodeEdit (Resize is not NodeEdit; the
    /// match is exhaustive on the variant tag, not a "modal-ish"
    /// catch-all).
    #[test]
    fn test_resolve_enter_section_edit_resize_mode_returns_not_in_node_edit() {
        use crate::application::app::interaction_mode::ResizeTarget;
        let (doc, id) = pinned_two_section_node();
        let mode = InteractionMode::Resize {
            target: ResizeTarget::Node(id),
        };
        let plan = resolve_enter_section_edit_plan(&mode, &doc.selection);
        assert_eq!(plan, EnterSectionEditPlan::NotInNodeEdit);
    }

    /// NodeEdit on node A + selection on node B → OwnerMismatch.
    /// The user steered selection to a sibling; SectionEdit on A
    /// would silently edit the wrong node.
    #[test]
    fn test_resolve_enter_section_edit_owner_mismatch() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single("other-node".to_string());
        let mode = InteractionMode::NodeEdit { node_id: id.clone() };
        let plan = resolve_enter_section_edit_plan(&mode, &doc.selection);
        assert_eq!(
            plan,
            EnterSectionEditPlan::OwnerMismatch {
                active_node: id,
                owner: Some("other-node".to_string()),
            }
        );
    }

    /// Edge selection → OwnerMismatch with `owner = None` (the
    /// edge has no node owner).
    #[test]
    fn test_resolve_enter_section_edit_edge_selection_returns_owner_mismatch_none() {
        let (mut doc, id) = pinned_two_section_node();
        let er = EdgeRef::new("a", "b", "cross_link");
        doc.selection = SelectionState::Edge(er);
        let mode = InteractionMode::NodeEdit { node_id: id.clone() };
        let plan = resolve_enter_section_edit_plan(&mode, &doc.selection);
        assert_eq!(
            plan,
            EnterSectionEditPlan::OwnerMismatch {
                active_node: id,
                owner: None,
            }
        );
    }

    /// NodeEdit on node A + Section selection on node A → OpenEditor.
    /// The happy path.
    #[test]
    fn test_resolve_enter_section_edit_section_selection_opens_editor() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel::new(&id, 1));
        let mode = InteractionMode::NodeEdit { node_id: id.clone() };
        let plan = resolve_enter_section_edit_plan(&mode, &doc.selection);
        assert_eq!(plan, EnterSectionEditPlan::OpenEditor { active_node: id });
    }

    /// NodeEdit + Single selection on the active node → OpenEditor
    /// (defaults to section 0 inside `open_text_edit`).
    #[test]
    fn test_resolve_enter_section_edit_single_on_active_node_opens_editor() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id.clone());
        let mode = InteractionMode::NodeEdit { node_id: id.clone() };
        let plan = resolve_enter_section_edit_plan(&mode, &doc.selection);
        assert_eq!(plan, EnterSectionEditPlan::OpenEditor { active_node: id });
    }

    /// `\n\n` not `\n` — single-newline collisions with
    /// intra-section editor line breaks are the whole reason the
    /// joiner moved to two newlines.
    #[test]
    fn test_multi_target_separator_is_doubled_newline() {
        assert_eq!(MULTI_TARGET_SEPARATOR, "\n\n");
    }

    /// Single-target paste always broadcasts verbatim (the split
    /// gate is `target_count > 1`). Pins that a single section
    /// receiving a `"foo\n\nbar"` blob writes the whole blob, not
    /// just `"foo"`.
    #[test]
    fn test_single_target_skips_split() {
        assert!(split_paste_for_targets("foo\n\nbar", 1).is_none());
        assert!(split_paste_for_targets("foo", 1).is_none());
    }

    /// Multi-target with matching fragment count zips: the round
    /// trip from a 3-target copy lands one fragment per target.
    #[test]
    fn test_matching_fragment_count_zips() {
        let frags = split_paste_for_targets("a\n\nb\n\nc", 3).expect("zip");
        assert_eq!(frags, vec!["a", "b", "c"]);
    }

    /// Mismatched fragment count broadcasts (e.g. cross-app paste
    /// of plain text into a multi-section selection).
    #[test]
    fn test_mismatched_fragment_count_broadcasts() {
        // 1 fragment, 3 targets — broadcast.
        assert!(split_paste_for_targets("plain text", 3).is_none());
        // 2 fragments, 3 targets — broadcast.
        assert!(split_paste_for_targets("a\n\nb", 3).is_none());
        // 4 fragments, 3 targets — broadcast.
        assert!(split_paste_for_targets("a\n\nb\n\nc\n\nd", 3).is_none());
    }

    /// Empty fragments are preserved by `str::split` — a copied
    /// section whose text is empty round-trips to an empty paste,
    /// not a dropped target.
    #[test]
    fn test_empty_fragment_preserved_in_zip() {
        let frags = split_paste_for_targets("a\n\n\n\nc", 3).expect("zip");
        assert_eq!(frags, vec!["a", "", "c"]);
    }

    /// **Stale-buffer guard.** A leftover single-section payload
    /// from a prior copy must not survive a multi-section copy —
    /// otherwise the structured paste path would silently
    /// substitute one section's payload for the joined OS-clipboard
    /// blob. Pins that `prepare_copy_or_cut` clears the in-process
    /// buffer up-front.
    #[test]
    fn test_multi_section_copy_clears_stale_section_buffer() {
        clear_section_clipboard();
        // Seed a stale buffer with a probe text the multi-section
        // copy below will NOT regenerate.
        let seed_payload = SectionPayload {
            text_runs: Vec::new(),
            offset: baumhard::mindmap::model::Position { x: 0.0, y: 0.0 },
            size: None,
            channel: None,
            trigger_bindings: Vec::new(),
        };
        write_section_clipboard("stale-probe".into(), seed_payload);
        assert!(read_section_clipboard("stale-probe").is_some());

        // Now do a MultiSection copy — the multi-section branch
        // doesn't write the structured buffer (no MultiSection
        // payload variant exists), so the only correctness gate
        // is the up-front clear.
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::MultiSection(vec![
            SectionSel::new(&id, 0),
            SectionSel::new(&id, 1),
        ]);
        let _ = prepare_copy_or_cut(false, &mut doc);

        assert!(
            read_section_clipboard("stale-probe").is_none(),
            "stale seeded buffer must be cleared by multi-section copy"
        );
    }

    /// Single-section copy DOES populate the structured buffer so
    /// within-app section→section paste round-trips per-run
    /// formatting. Pins that the up-front clear doesn't break the
    /// single-section structured path.
    ///
    /// Checks both layers: `prepare_copy_or_cut` returns a structured
    /// payload, and `apply_copy_or_cut` commits it to `SECTION_BUFFER`.
    #[test]
    fn test_single_section_copy_writes_structured_buffer() {
        // Computation layer: payload present in the returned value.
        clear_section_clipboard();
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel::new(&id, 1));
        let probe_text = doc.mindmap.nodes.get(&id).unwrap().sections[1].text.clone();
        let c = prepare_copy_or_cut(false, &mut doc).expect("section copy populates");
        assert_eq!(
            c.structured.as_ref().map(|s| s.text.as_str()),
            Some(probe_text.as_str()),
            "single-section copy must produce a structured payload"
        );

        // Wrapper layer: apply_copy_or_cut commits the payload to
        // SECTION_BUFFER so within-app paste can retrieve it.
        clear_section_clipboard();
        let (mut doc2, id2) = pinned_two_section_node();
        doc2.selection = SelectionState::Section(SectionSel::new(&id2, 1));
        let probe_text2 = doc2.mindmap.nodes.get(&id2).unwrap().sections[1].text.clone();
        apply_copy_or_cut(false, &mut doc2);
        assert!(
            read_section_clipboard(&probe_text2).is_some(),
            "apply_copy_or_cut must commit the structured payload to SECTION_BUFFER"
        );
    }

    /// MultiSection copy does NOT populate the structured buffer
    /// (no payload variant) — the within-app paste falls back to
    /// the OS clipboard's joined blob. Pins the
    /// no-payload-on-multi contract that the stale-buffer guard
    /// relies on.
    #[test]
    fn test_multi_section_copy_skips_structured_buffer() {
        clear_section_clipboard();
        let (mut doc, id) = pinned_two_section_node();
        let s0_text = doc.mindmap.nodes.get(&id).unwrap().sections[0].text.clone();
        doc.selection = SelectionState::MultiSection(vec![
            SectionSel::new(&id, 0),
            SectionSel::new(&id, 1),
        ]);
        let c = prepare_copy_or_cut(false, &mut doc).expect("multi-section copy populates");
        assert!(
            c.structured.is_none(),
            "multi-section copy must not produce a structured payload"
        );
        assert!(
            read_section_clipboard(&s0_text).is_none(),
            "multi-section copy must not write SECTION_BUFFER"
        );
    }

    /// **End-to-end multi-section round-trip.** Drive the per-target
    /// paste plumbing against a `MultiSection` selection and verify
    /// each section receives the correct fragment 1:1. Pins the
    /// split → zip → paste contract end-to-end (helper tests pin
    /// each leg in isolation; this fills the integration gap).
    ///
    /// The OS clipboard is bypassed deliberately. `arboard` is a
    /// process-global shared resource; writing to it from parallel
    /// tests made the original round-trip read flaky (~50% under the
    /// full file run, always-green under the debugger). The contract
    /// under test is `split_paste_for_targets` → zip → per-target
    /// `clipboard_paste` given a joined string; that string can be
    /// synthesised directly. The source-side
    /// `MULTI_TARGET_SEPARATOR` join is pinned by the helper-level
    /// `prepare_copy_or_cut` tests above.
    #[test]
    fn test_multi_section_copy_paste_round_trip() {
        use crate::application::console::traits::{
            selection_targets, view_for, HandlesPaste, Outcome,
        };

        clear_section_clipboard();
        let (mut doc, id) = pinned_two_section_node();

        // Pre-paste state: sentinel content that neither matches
        // the fragments nor sourced them. A no-op paste would
        // leave "wiped" in place and fail the assertions below.
        doc.mindmap.nodes.get_mut(&id).unwrap().sections[0].text = "wiped".into();
        doc.mindmap.nodes.get_mut(&id).unwrap().sections[1].text = "wiped".into();
        doc.selection = SelectionState::MultiSection(vec![
            SectionSel::new(&id, 0),
            SectionSel::new(&id, 1),
        ]);

        // Synthesize the joined clipboard string that
        // `apply_copy_or_cut` would have produced from sections
        // holding "alpha" / "beta". Asymmetric on purpose: a swap
        // or first-wins bug shows up in the per-section asserts.
        let text = ["alpha", "beta"].join(MULTI_TARGET_SEPARATOR);

        let targets = selection_targets(&doc.selection);
        assert_eq!(targets.len(), 2);
        let fragments = super::split_paste_for_targets(&text, targets.len())
            .expect("count matches → zip applies");
        for (i, tid) in targets.iter().enumerate() {
            let mut view = view_for(&mut doc, tid);
            assert_eq!(
                view.clipboard_paste(fragments[i]),
                Outcome::Applied,
                "per-target paste must apply"
            );
        }

        // Round-trip lands fragments 1:1.
        let s0 = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text;
        let s1 = &doc.mindmap.nodes.get(&id).unwrap().sections[1].text;
        assert_eq!(s0, "alpha", "section 0 must round-trip its own copy");
        assert_eq!(s1, "beta", "section 1 must round-trip its own copy");
    }

    /// `is_broadcast_paste` is `true` only when fragments couldn't be
    /// split per-target AND there are 2+ targets — the gate that
    /// triggers the structured-buffer clear inside `apply_paste`'s
    /// broadcast path. Without it, a stale single-section payload
    /// would apply its geometry to every target in a multi-target
    /// paste. Pin the predicate logic in isolation so changes to its
    /// definition are immediately visible.
    #[test]
    fn test_is_broadcast_paste_predicate() {
        use super::is_broadcast_paste;
        assert!(is_broadcast_paste(true, 2));
        assert!(is_broadcast_paste(true, 5));
        assert!(!is_broadcast_paste(true, 1));
        assert!(!is_broadcast_paste(false, 2));
        assert!(!is_broadcast_paste(false, 1));
    }
}
