// SPDX-License-Identifier: MPL-2.0

//! Picker lifecycle terminals: cancel, close-standalone, commit
//! (single-target / selection fan-out) and the hover-preview stamp
//! that feeds `doc.color_picker_preview` during mouse-move.

use crate::application::document::{EdgeRef, MindMapDocument};
use crate::application::renderer::Renderer;

use super::super::scene_rebuild::rebuild_all;
use super::super::throttled_interaction::ColorPickerHoverInteraction;

/// Cancel the picker: clear the transient document preview and
/// close the modal. The committed model is untouched because the
/// new preview path never writes to it — the entire hover / cancel
/// flow is a pure scene-level substitution.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn cancel_color_picker(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    interaction_mode: &super::super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    use crate::application::color_picker::ColorPickerState;

    if matches!(state, ColorPickerState::Closed) {
        return;
    }
    *state = ColorPickerState::Closed;
    doc.color_picker_preview = None;
    renderer.rebuild_color_picker_overlay_buffers(app_scene, None);
    rebuild_all(
        doc,
        interaction_mode,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
    );
}

/// Close the standalone color picker without committing. Called by
/// the `color picker off` console command. Functionally identical to
/// `cancel_color_picker` — both close the picker and clear the
/// transient preview — but named distinctly because Standalone mode
/// has no "original" to cancel back to; the function exists so
/// call-sites read clearly.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn close_color_picker_standalone(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    interaction_mode: &super::super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    cancel_color_picker(
        state,
        doc,
        interaction_mode,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
    );
}

/// Commit the picker's currently-previewed HSV value via the regular
/// `set_edge_color` / `set_node_*_color` path — a single undo entry
/// is pushed and `ensure_glyph_connection` runs its fork-on-first-edit
/// only at this moment (never during hover). Close the modal.
///
/// The picker only commits concrete HSV hex values now that the
/// theme-variable chip row has been retired; theme-variable editing
/// lives elsewhere in the UI.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn commit_color_picker(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    interaction_mode: &super::super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    use crate::application::color_picker::{ColorPickerState, NodeColorAxis, PickerHandle, SectionColorAxis};
    use baumhard::util::color::hsv_to_hex;

    let (handle, hue_deg, sat, val, seed_var_ref, seed_hsv) = match state {
        ColorPickerState::Open {
            mode:
                crate::application::color_picker::PickerMode::Contextual {
                    handle,
                    seed_var_ref,
                    seed_hsv,
                },
            hue_deg,
            sat,
            val,
            ..
        } => (
            handle.clone(),
            *hue_deg,
            *sat,
            *val,
            seed_var_ref.clone(),
            *seed_hsv,
        ),
        // Standalone mode has no bound target — commit is handled by
        // `commit_color_picker_to_selection` instead; this function
        // is Contextual-only. Being reached in Standalone mode means
        // the caller picked the wrong commit path.
        ColorPickerState::Open { .. } => {
            log::warn!(
                "commit_color_picker called in non-contextual mode; \
                 use commit_color_picker_to_selection for Standalone mode"
            );
            return;
        }
        ColorPickerState::Closed => return,
    };

    // Close the modal state first so the subsequent rebuilds don't
    // re-apply the preview.
    *state = ColorPickerState::Closed;
    doc.color_picker_preview = None;

    let hex = hsv_to_hex(hue_deg, sat, val);
    let to_write = pick_committed_value(seed_var_ref.as_deref(), seed_hsv, (hue_deg, sat, val), &hex);
    match handle {
        PickerHandle::Edge(index) => {
            let er = doc
                .mindmap
                .edges
                .get(index)
                .map(|e| EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type));
            if let Some(er) = er {
                doc.set_edge_color(&er, Some(&to_write));
            }
        }
        PickerHandle::Node { id, axis } => {
            let targets = node_commit_targets(&doc.selection, &id);
            for nid in &targets {
                match axis {
                    NodeColorAxis::Bg => {
                        doc.set_node_bg_color(nid, to_write.clone());
                    }
                    NodeColorAxis::Text => {
                        doc.set_node_text_color(nid, to_write.clone());
                    }
                    NodeColorAxis::Border => {
                        doc.set_node_border_color(nid, to_write.clone());
                    }
                }
            }
        }
        PickerHandle::Section {
            node_id,
            section_idx,
            axis,
            range,
        } => match axis {
            SectionColorAxis::Text => {
                // When the bound handle carries a sub-range, the
                // commit is range-targeted and skips the
                // MultiSection fan-out — sub-range semantics on
                // a multi-section selection don't compose
                // (different sections have different lengths).
                // Single-section / Section commit fans through
                // `section_commit_targets` as before.
                if let Some((rs, re)) = range {
                    let applied =
                        doc.set_section_text_color_range(&node_id, section_idx, rs, re, to_write.clone());
                    if !applied {
                        // Stale handle: section may have shrunk
                        // below `range_end`, or the node /
                        // section was deleted between picker
                        // open and commit. Surface a `log::warn!`
                        // so the user sees the keystroke didn't
                        // silently eat their commit.
                        log::warn!(
                            "color picker commit on section {} of node {} \
                             range {}..{} produced no change \
                             (section may have shrunk below the range \
                             or been deleted since picker open)",
                            section_idx,
                            node_id,
                            rs,
                            re
                        );
                    }
                } else {
                    let targets = section_commit_targets(&doc.selection, &node_id, section_idx);
                    for s in &targets {
                        doc.set_section_text_color(&s.node_id, s.section_idx, to_write.clone());
                    }
                }
            }
        },
    }

    renderer.rebuild_color_picker_overlay_buffers(app_scene, None);
    // `set_edge_color` / `set_node_*_color` mutate edge/node color
    // fields the connection pass caches per-edge (body glyph, color,
    // font). Clear so the rebuild re-samples against the committed
    // model.
    scene_cache.clear();
    rebuild_all(
        doc,
        interaction_mode,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
    );
}

/// Apply the current picker HSV to the document's transient color
/// preview, then rebuild only the scene (not the node tree, which
/// didn't change) + the picker overlay. Hot path: no ref resolution,
/// no model mutation, no snapshot. The scene builder reads the
/// preview via `doc.color_picker_preview` and substitutes it in
/// during emission.
///
/// Marks `picker_hover.dirty` so the per-frame throttle picks up
/// the change on its next drain — every mouse-move on the wheel
/// routes through here, and unguarded rebuilds would re-shape
/// every border / connection / portal on the map at ~120 Hz.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn apply_picker_preview(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    picker_hover: &mut ColorPickerHoverInteraction,
) {
    use crate::application::color_picker::{ColorPickerState, PickerHandle};
    use crate::application::document::ColorPickerPreview;
    use baumhard::util::color::hsv_to_hex;

    let (handle, eff_hue, eff_sat, eff_val) = match state {
        ColorPickerState::Open {
            mode,
            hue_deg,
            sat,
            val,
            hover_preview,
            ..
        } => {
            let handle = match mode {
                crate::application::color_picker::PickerMode::Contextual { handle, .. } => {
                    Some(handle.clone())
                }
                // Standalone mode has no bound target — nothing to
                // preview on the scene. The ࿕ glyph in the wheel
                // still shows the current HSV (rendered by the picker
                // overlay itself), so the user gets immediate
                // feedback without needing doc.color_picker_preview.
                crate::application::color_picker::PickerMode::Standalone => None,
            };
            let (eh, es, ev) = hover_preview.unwrap_or((*hue_deg, *sat, *val));
            (handle, eh, es, ev)
        }
        ColorPickerState::Closed => return,
    };
    let hex = hsv_to_hex(eff_hue, eff_sat, eff_val);
    if let Some(handle) = handle {
        match handle {
            PickerHandle::Edge(index) => {
                if let Some(edge) = doc.mindmap.edges.get(index) {
                    let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(edge);
                    doc.color_picker_preview = Some(ColorPickerPreview { key, color: hex });
                }
            }
            PickerHandle::Node { .. } => {
                // Node preview lives on the tree pipeline, not the
                // scene pipeline — not yet wired. Commit-only for v1.
            }
            PickerHandle::Section { .. } => {
                // Section text preview lives on the tree pipeline
                // alongside Node — not yet wired. Commit-only.
            }
        }
    }
    // Scene + picker rebuilds are deferred to the `AboutToWait`
    // drain via `picker_hover.dirty`. Mouse moves come in at
    // ~120Hz on modern hardware; without this gate every event
    // would re-shape every border / connection / portal on the
    // map plus the picker overlay. The drain is gated by
    // `picker_hover.throttle` (the same `MutationFrequencyThrottle`
    // type the drag path uses), which self-tunes to keep the
    // per-frame work under the refresh budget.
    picker_hover.mark_dirty();
    // Additionally flag the canvas dirty: `doc.color_picker_preview`
    // drives a per-edge color override that the scene builder reads
    // during emission. Only `apply_picker_preview` writes to that
    // preview — gesture-only paths (Move / Resize in `mouse.rs`)
    // leave it clear, which is what lets the drain skip
    // `rebuild_scene_only` during a wheel drag. Keyboard nudges,
    // however, land here even mid-drag; they must still trigger the
    // canvas rebuild so the targeted edge repaints.
    picker_hover.mark_canvas_dirty();
}

/// Commit the picker's current HSV to every colorable item in the
/// document's current selection. Standalone mode's core gesture.
///
/// Dispatches through the `AcceptsWheelColor` trait: each component
/// type declares its own default color channel (nodes → bg, edges →
/// their single color field). The picker doesn't decide — the
/// component does. Empty selection → fire the error-flash animation
/// hook and do nothing.
///
/// Multi-select applies in a single pass — one undo entry per item
/// (grouped undo is a future refinement when `UndoAction::Group`
/// lands in the document layer).
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn commit_color_picker_to_selection(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    interaction_mode: &super::super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    use crate::application::color_picker::{request_error_flash, ColorPickerState, FlashKind};
    use crate::application::console::traits::{
        selection_targets, view_for, AcceptsWheelColor, ColorValue, Outcome,
    };
    use baumhard::util::color::hsv_to_hex;

    let (hue_deg, sat, val) = match state {
        ColorPickerState::Open {
            hue_deg, sat, val, ..
        } => (*hue_deg, *sat, *val),
        ColorPickerState::Closed => return,
    };
    let color = ColorValue::Hex(hsv_to_hex(hue_deg, sat, val));

    let targets = selection_targets(&doc.selection);
    if targets.is_empty() {
        // The user pressed ࿕ with nothing selected. Fire the
        // animation hook (no-op stub today; picks up when the
        // animation pipeline lands) so the wheel flashes red.
        request_error_flash(state, FlashKind::Error);
        return;
    }

    // Fan out across the selection, letting each component decide
    // which channel the wheel color lands on. A fresh `TargetView`
    // per iteration so no two views alias the doc borrow.
    let mut any_accepted = false;
    for tid in &targets {
        let mut view = view_for(doc, tid);
        match view.apply_wheel_color(color.clone()) {
            Outcome::Applied | Outcome::Unchanged => any_accepted = true,
            Outcome::NotApplicable | Outcome::Invalid(_) => {}
        }
    }

    if any_accepted {
        // Same rationale as `commit_color_picker`: the wheel-color
        // writes land on cached edge fields, so clear before the
        // rebuild.
        scene_cache.clear();
        // Rebuild the whole scene so the newly-colored items repaint
        // next frame. The picker itself stays open — no state change
        // needed on `state`.
        rebuild_all(
            doc,
            interaction_mode,
            mindmap_tree,
            app_scene,
            renderer,
            scene_cache,
        );
    }
}

/// Bit-exact equality on a `(hue, sat, val)` triple. Compares
/// `f32::to_bits()` per channel so two HSV values that came from
/// the same source (e.g. seed-time `current_hsv_at` vs. an
/// untouched picker's still-seeded `(hue_deg, sat, val)`) test
/// equal even on the path where ordinary `f32` `==` would be
/// fragile under NaN. Used by `commit_color_picker` as the "did
/// the user move the wheel?" signal — anything that mutates
/// `(hue_deg, sat, val)` (cell click, keyboard nudge) flips the
/// answer; pure rendering or hover preview leaves it.
fn hsv_bits_equal(a: (f32, f32, f32), b: (f32, f32, f32)) -> bool {
    a.0.to_bits() == b.0.to_bits() && a.1.to_bits() == b.1.to_bits() && a.2.to_bits() == b.2.to_bits()
}

/// Compute the node-id list a `PickerHandle::Node` commit fans
/// out to. The picker handle binds to a single node at open time
/// (the first node in a `Multi` per
/// `commands/color::picker_target_outcome`); for a `Multi(ids)`
/// selection, the commit applies the chosen color to every
/// selected node. Single / Section / MultiSection / Edge / non-
/// node selections fall back to the bound handle's `id` — the
/// handle is authoritative when the current selection isn't a
/// multi-node set, even if the user changed selection between
/// open and commit.
///
/// `Multi` is dedup'd by id in first-seen order — `from_ids`
/// doesn't enforce uniqueness, and a stale dup would otherwise
/// produce a redundant setter call (idempotent on the color
/// value but doc-state churn / extra undo work).
pub(super) fn node_commit_targets(
    sel: &crate::application::document::SelectionState,
    handle_id: &str,
) -> Vec<String> {
    match sel {
        crate::application::document::SelectionState::Multi(ids) => {
            let mut seen = std::collections::HashSet::with_capacity(ids.len());
            let mut out = Vec::with_capacity(ids.len());
            for id in ids {
                if seen.insert(id.as_str()) {
                    out.push(id.clone());
                }
            }
            out
        }
        _ => vec![handle_id.to_string()],
    }
}

/// Compute the section list a `PickerHandle::Section` commit
/// fans out to. The handle binds to a single section at open
/// time (the first in a `MultiSection`); the current selection
/// drives fan-out — `MultiSection` writes to every entry,
/// `Section` writes to that one section, and any other selection
/// shape falls back to the bound handle.
///
/// **Handle-as-fallback union.** If the selection changed
/// between open and commit (user clicked elsewhere, then pressed
/// the picker's commit key) the bound handle's `(node_id,
/// section_idx)` is unioned in so the bound section never
/// silently drops out of the commit set. Dedup'd by
/// `(node_id, section_idx)` so the bound section that already
/// lives in the `MultiSection` set isn't written twice.
pub(super) fn section_commit_targets(
    sel: &crate::application::document::SelectionState,
    handle_node_id: &str,
    handle_section_idx: usize,
) -> Vec<crate::application::document::SectionSel> {
    use crate::application::document::{SectionSel, SelectionState};
    let mut out: Vec<SectionSel> = match sel {
        SelectionState::MultiSection(secs) => secs.clone(),
        SelectionState::Section(s) => vec![s.clone()],
        SelectionState::SectionRange { sel: s, .. } => vec![s.clone()],
        _ => Vec::new(),
    };
    let handle_target = SectionSel {
        node_id: handle_node_id.to_string(),
        section_idx: handle_section_idx,
    };
    if !out.iter().any(|s| s == &handle_target) {
        out.push(handle_target);
    }
    out
}

/// Decide the color string a Contextual picker commit writes.
/// When the user never moved the wheel from its open seed AND
/// the seed was a `var(--name)` reference, the reference is
/// preserved verbatim — otherwise the freshly-rendered hex from
/// the current HSV wins. Pure function so the var-preserve
/// invariant tests don't have to construct the full
/// Renderer/AppScene stack `commit_color_picker` needs.
pub(super) fn pick_committed_value(
    seed_var_ref: Option<&str>,
    seed_hsv: (f32, f32, f32),
    current_hsv: (f32, f32, f32),
    committed_hex: &str,
) -> String {
    if hsv_bits_equal(current_hsv, seed_hsv) {
        if let Some(raw) = seed_var_ref {
            return raw.to_string();
        }
    }
    committed_hex.to_string()
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::{node_commit_targets, pick_committed_value, section_commit_targets};
    use crate::application::document::{SectionSel, SelectionState};

    /// User never moved the wheel from its open seed AND the seed
    /// was a `var(--accent)` reference — commit preserves the
    /// reference verbatim instead of writing the resolved hex.
    /// Pins A1's primary semantics (Contextual mode).
    #[test]
    fn test_picker_commit_preserves_var_ref_when_unchanged() {
        let seed_hsv = (24.0_f32, 0.8_f32, 0.95_f32);
        let untouched = seed_hsv;
        let committed_hex = "#f3a020";
        let written = pick_committed_value(Some("var(--accent)"), seed_hsv, untouched, committed_hex);
        assert_eq!(
            written, "var(--accent)",
            "untouched picker on a var-ref seed must preserve the reference"
        );
    }

    /// User moved the wheel — commit writes the new hex even if
    /// the seed was a var ref. The reference is no longer "what
    /// the user picked"; honoring it would silently discard the
    /// new color.
    #[test]
    fn test_picker_commit_overwrites_var_ref_when_hue_moved() {
        let seed_hsv = (24.0_f32, 0.8_f32, 0.95_f32);
        let moved = (180.0_f32, 0.8_f32, 0.95_f32); // hue rotated
        let committed_hex = "#20a8f3";
        let written = pick_committed_value(Some("var(--accent)"), seed_hsv, moved, committed_hex);
        assert_eq!(
            written, "#20a8f3",
            "moved picker must write the new hex regardless of the seed's var ref"
        );
    }

    /// Plain-hex seed (no var ref) commits the new hex always —
    /// nothing to preserve, the unchanged-HSV case still writes
    /// the round-tripped hex (which is the same hex the seed
    /// would resolve to anyway, so the model field stays at its
    /// pre-open value modulo round-trip noise).
    #[test]
    fn test_picker_commit_writes_hex_when_no_var_ref() {
        let seed_hsv = (24.0_f32, 0.8_f32, 0.95_f32);
        let written_unchanged = pick_committed_value(None, seed_hsv, seed_hsv, "#f3a020");
        let written_moved = pick_committed_value(None, seed_hsv, (180.0, 0.8, 0.95), "#20a8f3");
        assert_eq!(written_unchanged, "#f3a020");
        assert_eq!(written_moved, "#20a8f3");
    }

    // ── Fan-out helpers ──────────────────────────────────────────

    /// `Multi(ids)` selection drives node-commit fan-out across
    /// every selected node. Pins the class-parallel fix: prior
    /// to N3.2 the node arm wrote only to the bound handle's id,
    /// silently dropping every other selected node.
    #[test]
    fn test_node_commit_targets_fans_out_for_multi_selection() {
        let sel = SelectionState::Multi(vec!["a".into(), "b".into(), "c".into()]);
        let targets = node_commit_targets(&sel, "a");
        assert_eq!(targets, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    }

    /// `Single(id)` selection writes to the bound handle (which
    /// equals the selected node by construction).
    #[test]
    fn test_node_commit_targets_uses_handle_for_single() {
        let sel = SelectionState::Single("a".into());
        let targets = node_commit_targets(&sel, "a");
        assert_eq!(targets, vec!["a".to_string()]);
    }

    /// **Handle-as-fallback.** If the user changed selection
    /// between picker open and commit (now `Section`, was
    /// `Single` when bound), the bound handle's id wins — the
    /// picker doesn't silently drop the color onto a node the
    /// user no longer has selected.
    #[test]
    fn test_node_commit_targets_falls_back_to_handle_when_selection_diverged() {
        let sel = SelectionState::Section(SectionSel::new("z", 0));
        let targets = node_commit_targets(&sel, "a");
        assert_eq!(targets, vec!["a".to_string()]);
    }

    /// `MultiSection` selection drives section-commit fan-out
    /// across every entry — pins the existing fan-out path.
    #[test]
    fn test_section_commit_targets_fans_out_for_multi_section() {
        let sel = SelectionState::MultiSection(vec![SectionSel::new("a", 0), SectionSel::new("b", 1)]);
        let targets = section_commit_targets(&sel, "a", 0);
        // Bound handle (a, 0) is already in the set — no dup.
        assert_eq!(targets, vec![SectionSel::new("a", 0), SectionSel::new("b", 1)]);
    }

    /// **Handle-union fix.** When the bound handle's section is
    /// NOT already in the `MultiSection` set (selection changed
    /// between open and commit), the handle is unioned in so the
    /// bound section never silently drops out of the commit.
    #[test]
    fn test_section_commit_targets_unions_handle_when_diverged() {
        let sel = SelectionState::MultiSection(vec![SectionSel::new("a", 0), SectionSel::new("b", 1)]);
        let targets = section_commit_targets(&sel, "c", 5);
        assert_eq!(
            targets,
            vec![
                SectionSel::new("a", 0),
                SectionSel::new("b", 1),
                SectionSel::new("c", 5),
            ]
        );
    }

    /// `Section(s)` selection collapses to a single section, the
    /// handle being the same section is dedup'd.
    #[test]
    fn test_section_commit_targets_section_selection_no_dup() {
        let sel = SelectionState::Section(SectionSel::new("a", 1));
        let targets = section_commit_targets(&sel, "a", 1);
        assert_eq!(targets, vec![SectionSel::new("a", 1)]);
    }

    /// Non-section / non-multi-section selections fall back to
    /// the bound handle alone.
    #[test]
    fn test_section_commit_targets_falls_back_to_handle_for_other_states() {
        let sel = SelectionState::None;
        let targets = section_commit_targets(&sel, "a", 1);
        assert_eq!(targets, vec![SectionSel::new("a", 1)]);

        let sel = SelectionState::Single("a".into());
        let targets = section_commit_targets(&sel, "a", 1);
        assert_eq!(targets, vec![SectionSel::new("a", 1)]);
    }
}
