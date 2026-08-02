// SPDX-License-Identifier: MPL-2.0

//! Per-node frame configuration: visibility flag plus the
//! `GlyphBorderConfig` overrides ([`BorderConfigEdits`]) that the
//! console's `border` verb stages and applies atomically. This file
//! owns the edit-bundle struct, the side selector, the
//! [`MindMapDocument`] setters that consume them, and the private
//! plumbing that folds a bundle into the `MindNode.style.border`
//! slot — including the auto-promotion of `preset` to `"custom"`
//! whenever a side- or corner-glyph edit is staged against a
//! built-in preset.
//!
//! Also home of the [`BorderPreview`] / [`BorderPreviewTarget`]
//! types and the three preview setters (`set_border_preview`,
//! `commit_border_preview`, `cancel_border_preview`) — the
//! live-preview surface for the four border verbs (per-node,
//! per-section, two canvas defaults). Same discipline as
//! `color_picker_preview`: never serialized, never push undo,
//! never flip `dirty`. Cancel / commit clears the slot
//! atomically; drift detection is lazy
//! (`border_preview_covers_live_selection`); implicit cancel
//! happens at the first line of each committing setter so a
//! non-preview edit always wins. The scene-build plumbing
//! lives in `frame_overrides`.

use baumhard::mindmap::border::PaletteField;
use baumhard::mindmap::border_pattern::SidePattern;
use baumhard::mindmap::model::GlyphBorderConfig;

use super::option_edit::{apply_option_edit, apply_string_set, apply_value_set, OptionEdit};
use super::{MindMapDocument, NodeEditTail, UndoAction};

/// Bundle of optional edits applied atomically by
/// [`MindMapDocument::set_node_border_config`]. Every field
/// defaults to "no edit"; the console verb composes one struct
/// per invocation and hands it to the setter.
///
/// Side-pattern fields carry pre-parsed [`SidePattern`]s plus the
/// raw input strings the parser produced from — the strings live
/// in the data model (so save / round-trip preserves the
/// original text) and the parsed payload validates the input
/// before the document is mutated. Construct with
/// [`BorderConfigEdits::with_side_pattern`] so a console caller
/// can't ship a parse-error string.
#[derive(Clone, Debug, Default)]
pub struct BorderConfigEdits {
    pub preset: OptionEdit<String>,
    pub font: OptionEdit<String>,
    pub font_size_pt: OptionEdit<f32>,
    pub color: OptionEdit<String>,
    pub padding: OptionEdit<f32>,
    pub color_palette: OptionEdit<String>,
    pub color_palette_field: OptionEdit<PaletteField>,
    pub side_top: OptionEdit<String>,
    pub side_bottom: OptionEdit<String>,
    pub side_left: OptionEdit<String>,
    pub side_right: OptionEdit<String>,
    pub corner_top_left: OptionEdit<String>,
    pub corner_top_right: OptionEdit<String>,
    pub corner_bottom_left: OptionEdit<String>,
    pub corner_bottom_right: OptionEdit<String>,
    /// `Some(true)` switches `style.show_frame` on, `Some(false)`
    /// off, `None` leaves it untouched. Kept on this struct (vs.
    /// a separate setter) so a single command can both flip the
    /// frame *and* configure it in one undoable step.
    pub visible: Option<bool>,
    /// `true` clears the per-node `style.border` override entirely
    /// (handles the `border reset` verb). When set, every other
    /// field on this struct is ignored.
    pub clear: bool,
}

impl BorderConfigEdits {
    /// Validate a side pattern string and stage it as a `Set`
    /// edit. Returns the parse error verbatim — the console verb
    /// surfaces it with a side prefix.
    pub fn with_side_pattern(&mut self, side: BorderSide, pattern: &str) -> Result<(), String> {
        SidePattern::parse(pattern).map_err(|e| format!("{}: {}", side.label(), e))?;
        let slot = match side {
            BorderSide::Top => &mut self.side_top,
            BorderSide::Bottom => &mut self.side_bottom,
            BorderSide::Left => &mut self.side_left,
            BorderSide::Right => &mut self.side_right,
        };
        *slot = OptionEdit::Set(pattern.to_string());
        Ok(())
    }
}

/// Side selector used by [`BorderConfigEdits::with_side_pattern`]
/// and the `border show` console output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderSide {
    Top,
    Bottom,
    Left,
    Right,
}

impl BorderSide {
    pub fn label(self) -> &'static str {
        match self {
            BorderSide::Top => "top",
            BorderSide::Bottom => "bottom",
            BorderSide::Left => "left",
            BorderSide::Right => "right",
        }
    }
}

/// Result of [`MindMapDocument::set_node_border_config`] —
/// distinguishes "no change" from "applied" and surfaces the
/// preset auto-promotion side effect so the console verb can
/// tell the user when their `preset=heavy top=…` request landed
/// as `preset=custom` (because setting any side or corner glyph
/// requires the custom preset for the data model to honor the
/// override at render time).
#[derive(Clone, Debug, Default)]
pub struct BorderEditOutcome {
    /// `true` when any field on the node actually changed.
    /// Console callers surface "no change" when this is false.
    pub changed: bool,
    /// `true` when the preset was auto-flipped from a non-custom
    /// value to `"custom"` because the same call also set a side
    /// or corner glyph. The verb's success message includes a
    /// note in that case.
    pub preset_auto_promoted: bool,
    /// The preset the user explicitly asked for in this edit, or
    /// `None` if no `preset=` kv was provided. Used together with
    /// `preset_auto_promoted` to phrase the auto-promotion note
    /// (`"preset=heavy was auto-promoted to 'custom'…"`).
    pub requested_preset: Option<String>,
}

/// Active live-preview substitution captured on
/// [`MindMapDocument::border_preview`]. The scene builder reads
/// this through a borrowed view (`tree_builder::BorderPreview<'a>`)
/// and substitutes the previewed `edits` for the resolved border
/// at the matching target. The model is never mutated; commit
/// dispatches to the matching committing setter and clears the
/// preview slot, cancel just clears.
///
/// `selection_snapshot` is the live selection at preview-set time —
/// the scene builder treats the preview as inactive when the
/// current `MindMapDocument.selection` no longer covers the
/// snapshot's targets (drift). The actual clear happens at the
/// next `set_*` / `commit_*` / `cancel_*` call; the steady-state
/// "orphaned by drift" preview is harmless until then.
///
/// One preview at a time: a fresh `set_border_preview` replaces
/// any active preview atomically.
#[derive(Clone, Debug)]
pub struct BorderPreview {
    pub target: BorderPreviewTarget,
    pub edits: BorderConfigEdits,
    pub selection_snapshot: SelectionState,
}

/// Which border slot the preview substitutes for at scene-build
/// time. Mirrors the four committing setters' shapes —
/// `Nodes(ids)` → [`MindMapDocument::set_node_border_config`],
/// `Sections((id, idx))` →
/// [`MindMapDocument::set_section_frame_border_config`],
/// `CanvasDefault` → [`MindMapDocument::set_canvas_default_border`],
/// `CanvasSectionFrame` / `CanvasSectionFrameFocused` →
/// [`MindMapDocument::set_canvas_default_section_frame_border_config`]
/// (with `focused = false / true`).
///
/// Single preview, single target variant — multi-target previews
/// (e.g. nodes *and* canvas-default at the same time) are
/// deliberately out of scope: setting a new preview replaces the
/// prior one.
#[derive(Clone, Debug)]
pub enum BorderPreviewTarget {
    Nodes(Vec<String>),
    Sections(Vec<(String, usize)>),
    CanvasDefault,
    CanvasSectionFrame,
    CanvasSectionFrameFocused,
}

impl MindMapDocument {
    /// Toggle the node's frame visibility. Returns `true` if the
    /// flag actually changed. No-op + no undo on no change, like
    /// every other style setter.
    ///
    /// Implicit-cancel rule: `border on` / `border off` against a
    /// node that's a target of an active per-node preview clears
    /// the preview before flipping `show_frame`. Without this, a
    /// preview's `force_show_frame` flag would keep rendering the
    /// staged border on top of a `border off` commit, so the
    /// user would see the border they just hid still on screen.
    /// Same scope-gating as `set_node_border_config`: only
    /// `Nodes(_)` / `CanvasDefault` previews cancel here; per-section
    /// and canvas-section-frame previews live in orthogonal
    /// surfaces and survive a node-visibility flip.
    ///
    /// Runs [`NodeEditTail::Border`] on commit, exactly as
    /// [`Self::set_node_border_config`] does: switching a frame
    /// on is the moment the node first needs room for its frame
    /// glyphs, and `grow_one_node_to_fit_border` no-ops on the
    /// off direction (it returns early when `show_frame` is
    /// false). Pre-fix `border on` alone skipped the grow while
    /// `border on preset=…` ran it — the same edit sized the node
    /// differently depending on whether an unrelated kv rode
    /// along.
    pub fn set_node_border_visible(&mut self, node_id: &str, on: bool) -> bool {
        if matches!(
            self.border_preview.as_ref().map(|p| &p.target),
            Some(BorderPreviewTarget::Nodes(_)) | Some(BorderPreviewTarget::CanvasDefault)
        ) {
            self.cancel_border_preview();
        }
        self.mutate_node_with_style_undo(node_id, NodeEditTail::Border, |node| {
            if node.style.show_frame == on {
                return None;
            }
            node.style.show_frame = on;
            Some(())
        })
        .is_some()
    }

    /// Apply a bundle of border edits to one node atomically.
    ///
    /// `edits.clear == true` drops the per-node `style.border`
    /// override entirely (the node falls back to the canvas
    /// default), ignoring every other field.
    ///
    /// Otherwise: every field with `OptionEdit::Set(v)` is
    /// written; `Clear` removes an existing override; `Keep`
    /// leaves the field untouched. Side-pattern strings are
    /// trusted to have been validated upstream
    /// (via [`BorderConfigEdits::with_side_pattern`]).
    ///
    /// After mutation, runs
    /// [`crate::application::document::grow_one_node_to_fit_border`] so
    /// the node grows to fit the new static parts; the same
    /// `EditNodeStyle` undo envelope captures both the style
    /// change and the size change.
    ///
    /// Returns a [`BorderEditOutcome`] describing whether
    /// anything changed and whether the preset was auto-promoted
    /// to `"custom"` (which happens whenever any side or corner
    /// glyph is set against a non-custom preset). The console
    /// verb surfaces the auto-promotion so the user knows their
    /// `preset=heavy top=…` request resulted in a `custom` border,
    /// not a `heavy` one with a side override.
    pub fn set_node_border_config(&mut self, node_id: &str, edits: BorderConfigEdits) -> BorderEditOutcome {
        // Scope-gated implicit cancel: a committing per-node edit
        // only clears previews whose visual scope it overlaps —
        // `Nodes(_)` previews target the same surface, and
        // `CanvasDefault` previews would render the canvas-default
        // through every framed node (this commit included). Other
        // preview kinds (per-section, canvas section-frame) live
        // in orthogonal visual surfaces and survive the per-node
        // commit. Pre-fix this cleared every preview unconditionally.
        if matches!(
            self.border_preview.as_ref().map(|p| &p.target),
            Some(BorderPreviewTarget::Nodes(_)) | Some(BorderPreviewTarget::CanvasDefault)
        ) {
            self.cancel_border_preview();
        }
        let mut outcome = BorderEditOutcome::default();
        // The border grow (`NodeEditTail::Border`) is monotonic
        // by design (mirrors `grow_node_sizes_to_fit_text`), so
        // undoing a border edit restores the style override but
        // leaves the node at its grown size — same posture as
        // text edits that grew the box. The user can shrink
        // manually if desired.
        let changed = self
            .mutate_node_with_style_undo(node_id, NodeEditTail::Border, |node| {
                let preset_before = node.style.border.as_ref().map(|c| c.preset.clone());
                let any_change = if edits.clear {
                    if node.style.border.is_none() && edits.visible.is_none() {
                        false
                    } else {
                        node.style.border = None;
                        if let Some(v) = edits.visible {
                            node.style.show_frame = v;
                        }
                        true
                    }
                } else {
                    apply_border_edits(node, &edits, &mut outcome)
                };
                if !any_change {
                    return None;
                }
                // Detect a preset auto-promotion to "custom" so
                // the verb's success message can tell the user
                // their explicit `preset=…` choice was overridden
                // by a side / corner edit. The user-asked-for
                // preset (if any) is captured up-front in
                // `outcome.requested_preset` by
                // `apply_border_edits`; here we compare against
                // what landed in the model.
                detect_preset_auto_promote(
                    node.style.border.as_ref(),
                    preset_before.as_deref(),
                    &mut outcome,
                );
                Some(())
            })
            .is_some();
        outcome.changed = changed;
        outcome
    }

    /// Apply a bundle of border edits to one section's
    /// `frame_border` atomically — the per-section equivalent of
    /// [`Self::set_node_border_config`]. Drives the
    /// `section frame …` console verb.
    ///
    /// `edits.clear == true` drops the per-section
    /// `frame_border` override (the section falls back to
    /// `Canvas.default_section_frame_border` and then to a
    /// hardcoded floor — same cascade as
    /// [`baumhard::mindmap::border::resolve_section_frame_border`]).
    /// `edits.visible` is ignored: section frames don't carry a
    /// per-frame visibility flag (NodeEdit drives their lifecycle).
    ///
    /// Returns the same [`BorderEditOutcome`] shape — the verb
    /// surfaces auto-promotion identically whether the edit
    /// landed on a node or a section.
    pub fn set_section_frame_border_config(
        &mut self,
        node_id: &str,
        section_idx: usize,
        edits: BorderConfigEdits,
    ) -> BorderEditOutcome {
        // Scope-gated implicit cancel — only clear previews whose
        // visual scope this commit overlaps: `Sections(_)` (same
        // per-section surface) and the two `CanvasSectionFrame*`
        // variants (which render through every section frame on
        // the active node). See `set_node_border_config` for the
        // rationale.
        if matches!(
            self.border_preview.as_ref().map(|p| &p.target),
            Some(BorderPreviewTarget::Sections(_))
                | Some(BorderPreviewTarget::CanvasSectionFrame)
                | Some(BorderPreviewTarget::CanvasSectionFrameFocused)
        ) {
            self.cancel_border_preview();
        }
        // Apply the staged edits to the section's frame_border
        // slot inside the shared envelope. The closure returns
        // its change verdict so the envelope decides whether to
        // push the undo entry + flip `dirty` — no post-hoc
        // `undo_stack.pop()` and no leaked `dirty=true` on no-ops.
        // `NodeEditTail::None`: a section frame renders inside the
        // node's AABB and never grows it.
        let mut outcome = BorderEditOutcome::default();
        let changed = self.mutate_section_with_style_undo(node_id, section_idx, NodeEditTail::None, |s| {
            let preset_before = s.frame_border.as_ref().map(|c| c.preset.clone());
            let any_change = if edits.clear {
                if s.frame_border.is_none() {
                    false
                } else {
                    s.frame_border = None;
                    true
                }
            } else {
                apply_glyph_border_edits_to_slot(&mut s.frame_border, &edits, &mut outcome)
            };
            if !any_change {
                return false;
            }
            // Detect preset auto-promotion (light / heavy /
            // etc. → custom) identically to the node-level
            // setter.
            detect_preset_auto_promote(s.frame_border.as_ref(), preset_before.as_deref(), &mut outcome);
            true
        });
        outcome.changed = changed;
        outcome
    }

    /// Apply a bundle of border edits to
    /// [`baumhard::mindmap::model::Canvas::default_border`] —
    /// the map-wide fallback every framed node falls back to when
    /// it has no per-node `style.border` override. Drives the
    /// `canvas border …` console verb.
    ///
    /// `edits.clear == true` drops the canvas default (every
    /// unframed node falls back to the hardcoded floor). `visible`
    /// is ignored: canvas-level defaults don't carry a visibility
    /// flag — the per-node `show_frame` toggle is the
    /// authoritative on/off.
    ///
    /// Captures the entire `Canvas` in a `CanvasSnapshot` undo
    /// entry so undo restores every theme / palette / default
    /// field in one step. Same posture as the
    /// `theme switch` verb.
    pub fn set_canvas_default_border(&mut self, edits: BorderConfigEdits) -> BorderEditOutcome {
        // Scope-gated implicit cancel — `CanvasDefault` previews
        // share the canvas-border slot directly; `Nodes(_)`
        // previews render against per-node-resolved styles which
        // include the canvas default in their cascade base.
        // Per-section / canvas-section-frame previews live in a
        // different visual surface and survive.
        if matches!(
            self.border_preview.as_ref().map(|p| &p.target),
            Some(BorderPreviewTarget::CanvasDefault) | Some(BorderPreviewTarget::Nodes(_))
        ) {
            self.cancel_border_preview();
        }
        let preset_before = self
            .mindmap
            .canvas
            .default_border
            .as_ref()
            .map(|c| c.preset.clone());
        let canvas_snapshot = self.mindmap.canvas.clone();
        let mut outcome = BorderEditOutcome::default();

        let any_change = if edits.clear {
            if self.mindmap.canvas.default_border.is_none() {
                false
            } else {
                self.mindmap.canvas.default_border = None;
                true
            }
        } else {
            apply_glyph_border_edits_to_slot(&mut self.mindmap.canvas.default_border, &edits, &mut outcome)
        };

        if !any_change {
            return outcome;
        }

        detect_preset_auto_promote(
            self.mindmap.canvas.default_border.as_ref(),
            preset_before.as_deref(),
            &mut outcome,
        );

        self.undo_stack.push(UndoAction::CanvasSnapshot {
            canvas: canvas_snapshot,
        });
        self.dirty = true;
        outcome.changed = true;
        outcome
    }

    /// Apply a bundle of border edits to either
    /// [`baumhard::mindmap::model::Canvas::default_section_frame_border`]
    /// (when `focused == false`) or
    /// [`baumhard::mindmap::model::Canvas::default_focused_section_frame_border`]
    /// (when `focused == true`). Drives the
    /// `canvas section-frame …` and `canvas section-frame focused …`
    /// console subverbs.
    ///
    /// Same `edits.clear` / `visible`-ignored / `CanvasSnapshot`
    /// undo / auto-promotion-detection contract as
    /// [`Self::set_canvas_default_border`].
    pub fn set_canvas_default_section_frame_border_config(
        &mut self,
        focused: bool,
        edits: BorderConfigEdits,
    ) -> BorderEditOutcome {
        // Scope-gated implicit cancel — clear previews whose
        // visual scope this commit overlaps:
        // `CanvasSectionFrame[Focused]` (same canvas slot) and
        // `Sections(_)` (each section frame composes the canvas
        // default into its cascade). Per-node / canvas-border
        // previews live in an orthogonal surface and survive.
        if matches!(
            self.border_preview.as_ref().map(|p| &p.target),
            Some(BorderPreviewTarget::CanvasSectionFrame)
                | Some(BorderPreviewTarget::CanvasSectionFrameFocused)
                | Some(BorderPreviewTarget::Sections(_))
        ) {
            self.cancel_border_preview();
        }
        let canvas_snapshot = self.mindmap.canvas.clone();
        let mut outcome = BorderEditOutcome::default();

        let preset_before = if focused {
            self.mindmap
                .canvas
                .default_focused_section_frame_border
                .as_ref()
                .map(|c| c.preset.clone())
        } else {
            self.mindmap
                .canvas
                .default_section_frame_border
                .as_ref()
                .map(|c| c.preset.clone())
        };

        let any_change = if edits.clear {
            let slot = if focused {
                &mut self.mindmap.canvas.default_focused_section_frame_border
            } else {
                &mut self.mindmap.canvas.default_section_frame_border
            };
            if slot.is_none() {
                false
            } else {
                *slot = None;
                true
            }
        } else {
            let slot = if focused {
                &mut self.mindmap.canvas.default_focused_section_frame_border
            } else {
                &mut self.mindmap.canvas.default_section_frame_border
            };
            apply_glyph_border_edits_to_slot(slot, &edits, &mut outcome)
        };

        if !any_change {
            return outcome;
        }

        let landed = if focused {
            self.mindmap.canvas.default_focused_section_frame_border.as_ref()
        } else {
            self.mindmap.canvas.default_section_frame_border.as_ref()
        };
        detect_preset_auto_promote(landed, preset_before.as_deref(), &mut outcome);

        self.undo_stack.push(UndoAction::CanvasSnapshot {
            canvas: canvas_snapshot,
        });
        self.dirty = true;
        outcome.changed = true;
        outcome
    }

    /// Set or replace the active border preview. Returns the
    /// outcome a *commit* would produce (`requested_preset`,
    /// `preset_auto_promoted`) computed by simulating
    /// `apply_glyph_border_edits_to_slot` against a clone of the
    /// affected slot — never re-implements the apply path. The
    /// console verb surfaces the simulated outcome so the user
    /// sees auto-promotion notes up-front, identical to what
    /// commit will report.
    ///
    /// No undo, no dirty, no model write. A prior preview is
    /// replaced atomically; orphan-by-drift previews are cleared.
    pub fn set_border_preview(
        &mut self,
        target: BorderPreviewTarget,
        edits: BorderConfigEdits,
    ) -> BorderEditOutcome {
        // Drop any orphan-by-drift preview before recording a new
        // one. Defer-clear posture: the scene-build path treats a
        // drifted preview as inactive; the actual slot empties
        // here on the next setter call. `cancel_border_preview`
        // and `commit_border_preview` open-code the same shape
        // because each wants a different *return* (false /
        // `None`) when drift was detected.
        if !self.border_preview_covers_live_selection() {
            self.border_preview = None;
        }
        // Simulate the apply against a clone of the affected slot
        // so the outcome reflects what commit will produce. Pick
        // the slot per target variant; for multi-target
        // (`Nodes(ids)` / `Sections(...)`), simulate against the
        // first target — auto-promotion is a property of `edits`,
        // not of the slot's pre-state, so any one slot is
        // representative. Empty target lists fall through to a
        // canvas-default-shaped slot just to drive the helper.
        let mut outcome = BorderEditOutcome::default();
        let mut slot_clone: Option<GlyphBorderConfig> = match &target {
            BorderPreviewTarget::Nodes(ids) => ids
                .first()
                .and_then(|id| self.mindmap.nodes.get(id))
                .and_then(|n| n.style.border.clone()),
            BorderPreviewTarget::Sections(pairs) => pairs
                .first()
                .and_then(|(id, idx)| self.mindmap.nodes.get(id).and_then(|n| n.sections.get(*idx)))
                .and_then(|s| s.frame_border.clone()),
            BorderPreviewTarget::CanvasDefault => self.mindmap.canvas.default_border.clone(),
            BorderPreviewTarget::CanvasSectionFrame => {
                self.mindmap.canvas.default_section_frame_border.clone()
            }
            BorderPreviewTarget::CanvasSectionFrameFocused => {
                self.mindmap.canvas.default_focused_section_frame_border.clone()
            }
        };
        apply_glyph_border_edits_to_slot(&mut slot_clone, &edits, &mut outcome);
        // The post-state preset on the cloned slot drives auto-
        // promotion detection; the same helper the four committing
        // setters use.
        let preset_before = match &target {
            BorderPreviewTarget::Nodes(ids) => ids
                .first()
                .and_then(|id| self.mindmap.nodes.get(id))
                .and_then(|n| n.style.border.as_ref())
                .map(|c| c.preset.clone()),
            BorderPreviewTarget::Sections(pairs) => pairs
                .first()
                .and_then(|(id, idx)| self.mindmap.nodes.get(id).and_then(|n| n.sections.get(*idx)))
                .and_then(|s| s.frame_border.as_ref())
                .map(|c| c.preset.clone()),
            BorderPreviewTarget::CanvasDefault => self
                .mindmap
                .canvas
                .default_border
                .as_ref()
                .map(|c| c.preset.clone()),
            BorderPreviewTarget::CanvasSectionFrame => self
                .mindmap
                .canvas
                .default_section_frame_border
                .as_ref()
                .map(|c| c.preset.clone()),
            BorderPreviewTarget::CanvasSectionFrameFocused => self
                .mindmap
                .canvas
                .default_focused_section_frame_border
                .as_ref()
                .map(|c| c.preset.clone()),
        };
        detect_preset_auto_promote(slot_clone.as_ref(), preset_before.as_deref(), &mut outcome);
        // The outcome's `changed` field is meaningful for commit
        // (where it gates the undo push); for preview-set we
        // surface it so the verb can say "no visible change" if
        // the staged edits are a no-op against the current slot.
        // The simulation already populated it via the helper.

        self.border_preview = Some(BorderPreview {
            target,
            edits,
            selection_snapshot: self.selection.clone(),
        });
        outcome
    }

    /// Discard any active preview. Returns `true` if a preview
    /// was active. O(1), no undo, no dirty — preview state is
    /// runtime-only.
    pub fn cancel_border_preview(&mut self) -> bool {
        // If the preview drifted before the cancel, treat it as
        // already-cleared so the bool reflects what the user
        // observed (no preview was rendering).
        if !self.border_preview_covers_live_selection() {
            self.border_preview = None;
            return false;
        }
        self.border_preview.take().is_some()
    }

    /// `true` iff the active preview's targets are still covered
    /// by the live selection. With no preview active, returns
    /// `true` (nothing to drift). Canvas-level previews never
    /// drift (they're not selection-bound).
    ///
    /// **Subset semantics, not equality.** `Multi(["A","B"]) →
    /// Single("A")` keeps `Nodes(["A"])` previews valid (A is
    /// still selected). `Section(A/0) → MultiSection([A/0,
    /// A/1])` keeps `Sections([(A,0)])` previews valid. Pre-fix
    /// this used `selection_snapshot == self.selection` which
    /// rejected state-preserving subtarget moves and produced
    /// false-positive "preview vanishes for no reason" UX bugs.
    /// Defer-clear posture: an orphan-by-drift preview just
    /// stops applying; the slot itself is cleared at the next
    /// `set_*` / `cancel_*` / `commit_*` call.
    pub(crate) fn border_preview_covers_live_selection(&self) -> bool {
        let Some(preview) = self.border_preview.as_ref() else {
            return true;
        };
        match &preview.target {
            BorderPreviewTarget::Nodes(target_ids) => {
                let live = live_selection_node_ids(&self.selection);
                target_ids.iter().all(|id| live.iter().any(|l| l == id))
            }
            BorderPreviewTarget::Sections(target_pairs) => {
                let live = live_selection_section_pairs(&self.selection);
                target_pairs.iter().all(|t| live.iter().any(|l| l == t))
            }
            // Canvas-level previews aren't selection-bound — they
            // affect map-wide defaults regardless of who's selected.
            BorderPreviewTarget::CanvasDefault
            | BorderPreviewTarget::CanvasSectionFrame
            | BorderPreviewTarget::CanvasSectionFrameFocused => true,
        }
    }

    /// Commit the active preview through the matching committing
    /// setter (`set_node_border_config` etc.) and clear the
    /// preview slot. Returns `Some(outcome)` when a preview was
    /// active (the outcome merges per-target results: first
    /// auto-promotion wins; `changed` is `true` if any underlying
    /// setter reported a change), `None` otherwise.
    ///
    /// Multi-node / multi-section commits push one undo entry per
    /// affected node — same posture as the verb-level
    /// `apply_edits` in `border/execute.rs`. Undoing a 5-node
    /// commit takes 5 Ctrl-Z's; intentional, not a regression.
    pub fn commit_border_preview(&mut self) -> Option<BorderEditOutcome> {
        // Drift = nothing to commit; treat as no-op.
        if !self.border_preview_covers_live_selection() {
            self.border_preview = None;
            return None;
        }
        let preview = self.border_preview.take()?;
        // Force-show coupling: when the preview's edits implied
        // visibility (any per-field edit, on a node whose
        // committed `show_frame == false`), the *preview* showed
        // the frame via the scene-side `force_show_frame` flag.
        // Without coupling that into commit, the user sees the
        // preview, hits commit, the frame disappears (the model's
        // `show_frame` stays false). For `Nodes(_)` targets we
        // therefore inject `visible = Set(true)` into the
        // commit-time edits whenever the preview's edits touch
        // any field — same predicate the scene-side preview used
        // to set `force_show_frame`. Per-section / canvas previews
        // don't carry a visibility axis (sections render in
        // NodeEdit unconditionally; canvas defaults don't have a
        // `show_frame` flag), so the coupling is `Nodes`-only.
        let mut commit_edits = preview.edits.clone();
        if matches!(preview.target, BorderPreviewTarget::Nodes(_)) && commit_edits.visible.is_none() {
            let touches_any_field = !matches!(commit_edits.preset, OptionEdit::Keep)
                || !matches!(commit_edits.font, OptionEdit::Keep)
                || !matches!(commit_edits.font_size_pt, OptionEdit::Keep)
                || !matches!(commit_edits.color, OptionEdit::Keep)
                || !matches!(commit_edits.padding, OptionEdit::Keep)
                || !matches!(commit_edits.color_palette, OptionEdit::Keep)
                || !matches!(commit_edits.color_palette_field, OptionEdit::Keep)
                || edits_touch_glyphs(&commit_edits);
            if touches_any_field || commit_edits.clear {
                commit_edits.visible = Some(true);
            }
        }
        let mut merged = BorderEditOutcome::default();
        match preview.target {
            BorderPreviewTarget::Nodes(ids) => {
                for id in &ids {
                    let outcome = self.set_node_border_config(id, commit_edits.clone());
                    merge_outcome(&mut merged, outcome);
                }
            }
            BorderPreviewTarget::Sections(pairs) => {
                for (id, idx) in &pairs {
                    let outcome = self.set_section_frame_border_config(id, *idx, commit_edits.clone());
                    merge_outcome(&mut merged, outcome);
                }
            }
            BorderPreviewTarget::CanvasDefault => {
                merged = self.set_canvas_default_border(commit_edits);
            }
            BorderPreviewTarget::CanvasSectionFrame => {
                merged = self.set_canvas_default_section_frame_border_config(false, commit_edits);
            }
            BorderPreviewTarget::CanvasSectionFrameFocused => {
                merged = self.set_canvas_default_section_frame_border_config(true, commit_edits);
            }
        }
        Some(merged)
    }
}

/// Fold one per-target outcome into the running merged outcome.
/// `changed` aggregates with OR; the first auto-promotion's
/// `requested_preset` wins (same posture as
/// `border/execute.rs::apply_edits`'s tally).
fn merge_outcome(merged: &mut BorderEditOutcome, one: BorderEditOutcome) {
    if one.changed {
        merged.changed = true;
    }
    if one.preset_auto_promoted && !merged.preset_auto_promoted {
        merged.preset_auto_promoted = true;
        merged.requested_preset = one.requested_preset;
    }
}

/// Set `outcome.preset_auto_promoted = true` when `landed`'s preset
/// is `"custom"` (case-insensitive), the pre-edit preset wasn't
/// already custom, and the user explicitly asked for some preset
/// (i.e. their kv mentioned `preset=`). Shared between the four
/// border-style setters (`set_node_border_config`,
/// `set_section_frame_border_config`,
/// `set_canvas_default_border`,
/// `set_canvas_default_section_frame_border_config`) so the
/// detection logic lives in exactly one place. When the auto-
/// promote path is removed (per `SECTIONS_BORDERS_RESIZE_PLAN.md`
/// §5.4), this function and the `BorderEditOutcome.preset_auto_promoted`
/// field go together.
pub(super) fn detect_preset_auto_promote(
    landed: Option<&GlyphBorderConfig>,
    preset_before: Option<&str>,
    outcome: &mut BorderEditOutcome,
) {
    let Some(cfg) = landed else { return };
    if !cfg.preset.eq_ignore_ascii_case("custom") {
        return;
    }
    let was_already_custom = preset_before
        .map(|p| p.eq_ignore_ascii_case("custom"))
        .unwrap_or(false);
    if !was_already_custom && outcome.requested_preset.is_some() {
        outcome.preset_auto_promoted = true;
    }
}

/// Apply non-clear edits to a node's style/border. Returns
/// `true` when at least one slot actually changed value (so the
/// caller can decide whether to push an undo entry).
///
/// Bookkeeping is one boolean we OR with each per-field check —
/// avoids an `EditEqOp` clone-and-compare on the whole NodeStyle
/// (which doesn't implement `PartialEq`) and also lets a no-op
/// kv pair like `border on` against an already-on border short-
/// circuit cleanly.
pub(super) fn apply_border_edits(
    node: &mut baumhard::mindmap::model::MindNode,
    edits: &BorderConfigEdits,
    outcome: &mut BorderEditOutcome,
) -> bool {
    let mut changed = false;
    if let Some(v) = edits.visible {
        if node.style.show_frame != v {
            node.style.show_frame = v;
            changed = true;
        }
    }
    changed |= apply_glyph_border_edits_to_slot(&mut node.style.border, edits, outcome);
    changed
}

/// Slot-level helper that applies every config-side field on
/// `BorderConfigEdits` (preset / font / size / color / padding /
/// palette / field / sides / corners) directly to a
/// `&mut Option<GlyphBorderConfig>`. Skips the `visible` flag —
/// that's a node-only concept that the per-node wrapper layers on
/// top.
///
/// Shared between `apply_border_edits` (writes `node.style.border`)
/// and [`MindMapDocument::set_section_frame_border_config`] (writes
/// `section.frame_border`). The factoring is what lets the
/// `border …` and `section frame …` verbs feed the same kv
/// vocabulary into two different model slots.
///
/// `outcome.requested_preset` is set when the caller passed
/// `preset=…` so the upper layer can phrase the "auto-promoted to
/// custom" message after detecting the preset shift.
pub(crate) fn apply_glyph_border_edits_to_slot(
    slot: &mut Option<GlyphBorderConfig>,
    edits: &BorderConfigEdits,
    outcome: &mut BorderEditOutcome,
) -> bool {
    if let OptionEdit::Set(p) = &edits.preset {
        outcome.requested_preset = Some(p.clone());
    }

    // Skip the slot allocation entirely when no config-side field
    // was touched. The caller may still have written `visible`
    // before us; that change is its bookkeeping, not ours.
    if !edits_touch_cfg_field(edits) {
        return false;
    }

    let mut changed = false;
    let had_cfg = slot.is_some();
    let cfg = slot.get_or_insert_with(default_glyph_border_config);
    if !had_cfg {
        changed = true;
    }

    if let OptionEdit::Set(p) = &edits.preset {
        if cfg.preset != *p {
            cfg.preset = p.clone();
            changed = true;
        }
    }
    changed |= apply_option_edit(&edits.font, &mut cfg.font, |v| v.clone());
    changed |= apply_value_set(&edits.font_size_pt, &mut cfg.font_size_pt);
    changed |= apply_option_edit(&edits.color, &mut cfg.color, |v| v.clone());
    changed |= apply_value_set(&edits.padding, &mut cfg.padding);
    changed |= apply_option_edit(&edits.color_palette, &mut cfg.color_palette, |v| v.clone());
    changed |= apply_option_edit(&edits.color_palette_field, &mut cfg.color_palette_field, |v| {
        v.as_str().to_string()
    });

    // Sides + corners: ensure the `glyphs` slot exists if any of
    // the eight glyph-string fields is being edited. The schema
    // says they're consulted only when `preset == "custom"`, so
    // setting a side without flipping the preset is silently a
    // no-op visually — the console verb upgrades the preset for
    // the user when any side / corner is set.
    if edits_touch_glyphs(edits) {
        if cfg.glyphs.is_none() {
            cfg.glyphs = Some(default_custom_glyphs());
            changed = true;
        }
        if !preset_is_custom(&cfg.preset) {
            cfg.preset = "custom".to_string();
            changed = true;
        }
        let g = cfg.glyphs.as_mut().expect("just inserted");
        changed |= apply_string_set(&edits.side_top, &mut g.top);
        changed |= apply_string_set(&edits.side_bottom, &mut g.bottom);
        changed |= apply_string_set(&edits.side_left, &mut g.left);
        changed |= apply_string_set(&edits.side_right, &mut g.right);
        changed |= apply_string_set(&edits.corner_top_left, &mut g.top_left);
        changed |= apply_string_set(&edits.corner_top_right, &mut g.top_right);
        changed |= apply_string_set(&edits.corner_bottom_left, &mut g.bottom_left);
        changed |= apply_string_set(&edits.corner_bottom_right, &mut g.bottom_right);
    }
    changed
}

fn edits_touch_cfg_field(edits: &BorderConfigEdits) -> bool {
    !matches!(edits.preset, OptionEdit::Keep)
        || !matches!(edits.font, OptionEdit::Keep)
        || !matches!(edits.font_size_pt, OptionEdit::Keep)
        || !matches!(edits.color, OptionEdit::Keep)
        || !matches!(edits.padding, OptionEdit::Keep)
        || !matches!(edits.color_palette, OptionEdit::Keep)
        || !matches!(edits.color_palette_field, OptionEdit::Keep)
        || edits_touch_glyphs(edits)
}

fn edits_touch_glyphs(edits: &BorderConfigEdits) -> bool {
    matches!(edits.side_top, OptionEdit::Set(_))
        || matches!(edits.side_bottom, OptionEdit::Set(_))
        || matches!(edits.side_left, OptionEdit::Set(_))
        || matches!(edits.side_right, OptionEdit::Set(_))
        || matches!(edits.corner_top_left, OptionEdit::Set(_))
        || matches!(edits.corner_top_right, OptionEdit::Set(_))
        || matches!(edits.corner_bottom_left, OptionEdit::Set(_))
        || matches!(edits.corner_bottom_right, OptionEdit::Set(_))
}

fn preset_is_custom(s: &str) -> bool {
    s.eq_ignore_ascii_case("custom")
}

use baumhard::mindmap::border::default_custom_glyphs;
use baumhard::mindmap::border::default_glyph_border_config;

/// Resolve the live selection's set of node ids — the same shape
/// `border_preview_covers_live_selection` uses to compare against
/// a `Nodes(ids)` snapshot. `Section` / `SectionRange` /
/// `MultiSection` collapse to their owning node ids; non-node
/// selections (edges / portal labels / etc.) yield an empty list,
/// causing any node-targeted preview to read as drifted.
use crate::application::document::SelectionState;

fn live_selection_node_ids(sel: &SelectionState) -> Vec<String> {
    match sel {
        SelectionState::Single(id) => vec![id.clone()],
        SelectionState::Multi(ids) => ids.clone(),
        SelectionState::Section(s) => vec![s.node_id.clone()],
        SelectionState::SectionRange { sel: s, .. } => vec![s.node_id.clone()],
        SelectionState::MultiSection(_) => sel.dedup_owning_node_ids(),
        _ => Vec::new(),
    }
}

/// Resolve the live selection's set of `(node_id, section_idx)`
/// pairs — the same shape
/// `border_preview_covers_live_selection` uses against a
/// `Sections(...)` snapshot. `SectionRange` expands; `MultiSection`
/// fans out; non-section selections yield an empty list.
fn live_selection_section_pairs(sel: &SelectionState) -> Vec<(String, usize)> {
    match sel {
        SelectionState::Section(s) => vec![(s.node_id.clone(), s.section_idx)],
        SelectionState::SectionRange { sel, range } => {
            let (lo, hi) = (range.0.min(range.1), range.0.max(range.1));
            (lo..=hi).map(|i| (sel.node_id.clone(), i)).collect()
        }
        SelectionState::MultiSection(sels) => {
            sels.iter().map(|s| (s.node_id.clone(), s.section_idx)).collect()
        }
        _ => Vec::new(),
    }
}
