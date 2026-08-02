// SPDX-License-Identifier: MPL-2.0

//! View-side override inputs for the per-role tree projection.
//!
//! Every builder under [`crate::mindmap::tree_builder`] projects
//! the *committed* model. These types carry the transient,
//! frame-local substitutions the application layer folds in on top
//! of it: in-flight color-picker hovers, staged border-preview
//! edits, the active selection, and the inline text editors'
//! uncommitted buffers. None of them ever reaches the persisted
//! `MindMap` — the projection reads them, the model never does.
//!
//! They live together because they share one lifetime posture: the
//! application layer owns the data and threads a borrow into a
//! per-frame build call, so every type here is `Copy`-cheap and
//! borrows rather than owns.

use crate::mindmap::scene_cache::EdgeKey;

use super::portal_style::SelectedPortalLabel;

/// A transient, scene-build-only substitution of an edge's effective
/// color. Used by the inline color picker's hover preview so the edge
/// under the wheel reflects the in-flight HSV value **without** any
/// mutation to the committed model. One edge at a time (the picker is
/// modal) so a single Option is enough.
///
/// Applied after the normal "glyph_connection.color → edge.color →
/// canvas default" resolution path but **before** the selection
/// override, so a selected edge being previewed still renders cyan on
/// the body glyphs. The preview is visible on the connection label,
/// matching the pre-refactor behavior.
#[derive(Debug, Clone, Copy)]
pub struct EdgeColorPreview<'a> {
    pub edge_key: &'a EdgeKey,
    pub color: &'a str,
}

/// View-side overrides telling the scene builder which node /
/// section should receive mode-driven chrome this frame: resize
/// handles on the active resize target, and inactive-node dimming
/// when NodeEdit is open. Computed by the application layer
/// (translating from its interaction-mode state) and threaded into
/// [`FrameOverrides`] and consumed by the border, section-frame,
/// and handle passes.
///
/// `Default` is no handles + no dimming. Pre-Batch-2 of the
/// sections / borders / resize UX overhaul, the scene builder read
/// selection directly (`Single` → handles, `Section` → handles),
/// which produced the "accidental resize on selection" UX bug.
/// Decoupling the gate from selection — and putting it next to its
/// consumer `SceneSelectionContext` — keeps the model/view boundary
/// clean: the document doesn't know about modes, the app translates
/// mode to override, the scene builder consumes the override.
///
/// One bundle per rebuild — adding a third mode-derived override
/// (e.g. `Resize`-mode body tinting) extends the struct rather than
/// threading another parameter through every pass signature.
#[derive(Debug, Default, Clone, Copy)]
pub struct InteractionModeOverrides<'a> {
    /// Which node should auto-emit 8 resize handles this frame, or
    /// `None` for no node handles.
    pub node: Option<&'a str>,
    /// Which section (`(node_id, section_idx)`) should auto-emit 8
    /// resize handles, or `None` for no section handles. Sections
    /// with `size == None` (fill-parent) emit zero handles inside
    /// the builder regardless — there's no own AABB to stretch.
    pub section: Option<(&'a str, usize)>,
    /// Active NodeEdit target. When `Some(active)`, every node other
    /// than `active` renders chrome + text at the inactive-alpha
    /// multiplier (see
    /// [`super::INACTIVE_NODE_ALPHA_MULTIPLIER`])
    /// — the "you are inside this node" affordance. `None` (the
    /// Default-mode case) is the no-op fast path.
    pub node_edit_for: Option<&'a str>,
    /// Section currently inside the inline text editor, if any.
    /// `Some((node_id, section_idx))` causes the matching
    /// `SectionFrameElement` to emit `focused = true` so the
    /// renderer draws its perimeter at a thicker stroke (Plan
    /// §4.4). `None` is the no-op — every emitted frame draws at
    /// the standard stroke. Read by the section-frame builder via
    /// [`SceneSelectionContext::focused_section`].
    pub focused_section: Option<(&'a str, usize)>,
}

impl<'a> InteractionModeOverrides<'a> {
    /// All-`None` overrides — equivalent to `Default::default()`
    /// but named for clarity at construction sites that want to
    /// be explicit about "this rebuild emits no handles".
    pub const fn none() -> Self {
        Self {
            node: None,
            section: None,
            node_edit_for: None,
            focused_section: None,
        }
    }
}

/// Portal equivalent of `EdgeColorPreview`. Matched against the
/// portal-mode edge's `EdgeKey`. A portal-mode edge and a line-mode
/// edge with identical endpoints and `edge_type` would share the
/// same key; since `display_mode` is not part of `EdgeKey`, that
/// collision never occurs in practice — portal and line edges with
/// matching endpoints are distinct by `edge_type`.
#[derive(Debug, Clone, Copy)]
pub struct PortalColorPreview<'a> {
    pub edge_key: &'a EdgeKey,
    pub color: &'a str,
}

/// Transient, scene-build-only substitution of a border's resolved
/// configuration. Drives the `border preview …` /
/// `section frame preview …` / `canvas border preview …` /
/// `canvas section-frame [focused] preview …` console verbs.
///
/// While `Some(...)` is threaded through the build pipeline, the
/// scene builder folds the previewed `edits` into a clone of the
/// committed slot at the matching target before resolution — the
/// committed model in `MindMap` is never mutated; this preview is
/// purely a scene-level substitution. Borrow shape mirrors
/// [`EdgeColorPreview`] / [`PortalColorPreview`]: the application
/// layer owns the data, threads a borrow into the scene call.
///
/// `force_show_frame` lets a preview of **any** field — the
/// shape-changing ones like `border preview preset=heavy`, but
/// equally `border preview color=red` — render against a node
/// whose committed `style.show_frame == false`. Without it the
/// preview would be invisible and the user would think the verb
/// was broken. Commit writes the explicit visibility flip through
/// the normal setter, so the force flag never leaves the
/// projection.
#[derive(Debug, Clone, Copy)]
pub struct BorderPreview<'a> {
    pub target: BorderPreviewTargetRef<'a>,
    /// View carried by value — it's already a borrow of the
    /// document's `BorderConfigEdits`, so cloning it just copies
    /// 17 fields of `Option<&str>` / `Option<f32>` / `bool`. No
    /// secondary borrow needed.
    pub edits: BorderConfigEditsView<'a>,
    pub force_show_frame: bool,
}

/// Borrowed view of the document-side `BorderPreviewTarget`. The
/// scene builder reads through these slices without taking
/// ownership of the doc's `Vec`s.
#[derive(Debug, Clone, Copy)]
pub enum BorderPreviewTargetRef<'a> {
    Nodes(&'a [String]),
    Sections(&'a [(String, usize)]),
    CanvasDefault,
    CanvasSectionFrame,
    CanvasSectionFrameFocused,
}

/// Per-field tri-state edit, mirroring the application crate's
/// `OptionEdit<T>` (`Keep` / `Clear` / `Set`). The scene-side
/// view carries this so `OptionEdit::Clear` round-trips into the
/// preview pipeline — pre-fix `BorderConfigEditsView` collapsed
/// `Clear` to "no edit" and the rendered preview diverged from
/// what commit produced (Risk #1 in the plan).
///
/// `Keep` = no edit (steady-state default); `Clear` = drop the
/// field on the slot; `Set(v)` = write the borrowed value.
/// `Default` is `Keep` so a `BorderConfigEditsView::default()`
/// is a no-op view.
#[derive(Debug, Clone, Copy, Default)]
pub enum EditView<T: Copy> {
    #[default]
    Keep,
    Clear,
    Set(T),
}

impl<T: Copy> EditView<T> {
    /// `true` iff the edit is `Set` or `Clear` — i.e. it touches
    /// the field, vs `Keep` which leaves it alone. Used by the
    /// "any field touched?" predicates that gate slot allocation
    /// + force-show-frame logic.
    pub fn is_edit(&self) -> bool {
        !matches!(self, EditView::Keep)
    }
}

/// Scene-side mirror of the application-crate `BorderConfigEdits`
/// struct. The application crate owns `BorderConfigEdits` (it
/// imports `OptionEdit` and shapes around the document layer);
/// this view exposes just the resolved option-fields the slot
/// helper needs at scene-build time. The application layer
/// constructs an instance from the owned `BorderConfigEdits` and
/// hands the borrow into [`BorderPreview`].
///
/// Mirrors the slot-helper's read shape — preset / font / size /
/// color / palette / palette_field / padding / four sides / four
/// corners — each as an [`EditView`] tri-state so that
/// `OptionEdit::Clear` survives the projection. Plus a top-level
/// `clear: bool` that empties the entire slot (mirrors
/// `BorderConfigEdits.clear`).
#[derive(Debug, Clone, Copy, Default)]
pub struct BorderConfigEditsView<'a> {
    pub preset: EditView<&'a str>,
    pub font: EditView<&'a str>,
    pub font_size_pt: EditView<f32>,
    pub color: EditView<&'a str>,
    pub padding: EditView<f32>,
    pub color_palette: EditView<&'a str>,
    pub color_palette_field: EditView<&'a str>,
    pub side_top: EditView<&'a str>,
    pub side_bottom: EditView<&'a str>,
    pub side_left: EditView<&'a str>,
    pub side_right: EditView<&'a str>,
    pub corner_top_left: EditView<&'a str>,
    pub corner_top_right: EditView<&'a str>,
    pub corner_bottom_left: EditView<&'a str>,
    pub corner_bottom_right: EditView<&'a str>,
    /// `true` clears the slot entirely (the cascade falls through
    /// to the canvas default or the hardcoded floor). Mirrors
    /// `BorderConfigEdits.clear`.
    pub clear: bool,
}

impl<'a> BorderConfigEditsView<'a> {
    /// `true` iff any per-field axis is `Set` or `Clear`. Used by
    /// the slot-allocation gate inside `apply_view_to_slot` and
    /// by the force-show-frame predicate (along with `clear`,
    /// which is its own axis).
    pub fn touches_any_field(&self) -> bool {
        self.preset.is_edit()
            || self.font.is_edit()
            || self.font_size_pt.is_edit()
            || self.color.is_edit()
            || self.padding.is_edit()
            || self.color_palette.is_edit()
            || self.color_palette_field.is_edit()
            || self.touches_glyphs()
    }

    /// `true` iff any side- or corner-glyph axis is `Set` or
    /// `Clear`. Mirrors the application-side `edits_touch_glyphs`
    /// predicate; lifts the eight-way OR into the type so the
    /// app side can drop its parallel copy.
    pub fn touches_glyphs(&self) -> bool {
        self.side_top.is_edit()
            || self.side_bottom.is_edit()
            || self.side_left.is_edit()
            || self.side_right.is_edit()
            || self.corner_top_left.is_edit()
            || self.corner_top_right.is_edit()
            || self.corner_bottom_left.is_edit()
            || self.corner_bottom_right.is_edit()
    }
}

/// Bundle of "what is the user currently pointing at?" inputs
/// threaded into the scene build. Groups the three selection-
/// like overrides (whole-edge select, per-label select, inline
/// label-edit substitution) so the per-role pass signatures stay
/// readable; the in-flight color previews stay separate because
/// they're hover-state, not selection-state.
///
/// Empty context (all three fields `None`) is the common case —
/// use [`SceneSelectionContext::default`] instead of spelling
/// out `SceneSelectionContext { edge: None, .. }` at call sites.
#[derive(Debug, Clone, Default)]
pub struct SceneSelectionContext<'a> {
    /// Whole edge selection — applies the cyan highlight to both
    /// markers of a portal-mode edge (or the body glyphs of a
    /// line-mode edge). Tuple is `(from_id, to_id, edge_type)`.
    pub edge: Option<(&'a str, &'a str, &'a str)>,
    /// Edge-label sub-selection — applies the cyan highlight to
    /// just the line-mode label text for the named edge, without
    /// tinting the body glyphs. Set by `SelectionState::EdgeLabel`;
    /// mutually exclusive with `edge` by construction on the caller
    /// side (`SelectionState` is an enum). Distinct from `edge` so
    /// clicking just the label tints only the label, matching what
    /// the user pointed at.
    ///
    /// Stored by value (not as a borrow) because `EdgeLabelSel`
    /// holds an `EdgeRef` — three strings, which the context
    /// assembly at the document layer converts into an `EdgeKey`
    /// per call. The cost is three `String` clones; negligible
    /// next to the per-frame scene build.
    pub edge_label: Option<EdgeKey>,
    /// Per-label selection — applies the cyan highlight to just
    /// one endpoint's marker on a portal-mode edge. Mutually
    /// exclusive with `edge` by construction on the caller side
    /// (`SelectionState` is an enum).
    pub portal_label: Option<SelectedPortalLabel<'a>>,
    /// Inline edge-label editor override — substitutes the
    /// in-progress buffer + caret for the committed label text
    /// on the named edge, so label edits render live.
    pub label_edit: Option<(&'a EdgeKey, &'a str)>,
    /// Selected section identity — `(node_id, section_idx)` —
    /// driving section-resize-handle emission. When `Some` and
    /// the named section has `Some` size, the scene includes 8
    /// handles for the section. `None` (the default) emits no
    /// section handles.
    pub selected_section: Option<(&'a str, usize)>,
    /// Selected node identity for node-resize-handle emission.
    /// Set by callers when the selection is `Single(node_id)`;
    /// empty otherwise. The scene includes 8 handles for the
    /// node when its size is finite + positive.
    pub selected_node_for_resize: Option<&'a str>,
    /// Active NodeEdit target. When `Some(active)`, every node
    /// other than `active` renders chrome + text at the inactive-
    /// alpha multiplier (see
    /// [`super::INACTIVE_NODE_ALPHA_MULTIPLIER`]) — the "you are
    /// inside this node" affordance for
    /// `InteractionMode::NodeEdit`. `None` (the Default-mode
    /// case) is the no-op fast path: every node draws at full
    /// opacity. Set from the application layer at the same call
    /// site that fills `selected_node_for_resize` /
    /// `selected_section`; the frame half routes through
    /// [`super::border_node_data`] and the text half through the
    /// app crate's `apply_inactive_node_dimming` overlay.
    pub node_edit_for: Option<&'a str>,
    /// Section currently inside the inline text editor, if any.
    /// Read by the section-frame pass to mark the matching
    /// `SectionFrameElement.focused = true`. `None` = no editor
    /// open or the editor's section isn't part of an emitted frame
    /// set (Default mode, single-section node).
    pub focused_section: Option<(&'a str, usize)>,
}

/// Substitution pair for the portal-text inline edit preview.
/// Carries the `(edge_key, endpoint_node_id)` identity of the
/// target portal label plus the current buffer contents to be
/// rendered in place of the committed `PortalEndpointState.text`.
/// Consumed by [`super::portal_pair_data`], which substitutes it
/// for the committed `PortalEndpointState.text` on the named
/// endpoint.
#[derive(Debug, Clone, Copy)]
pub struct PortalTextEditOverride<'a> {
    pub edge_key: &'a EdgeKey,
    pub endpoint_node_id: &'a str,
    pub buffer: &'a str,
}

/// Every frame-local override the per-role passes read, in one
/// borrow. The application layer assembles this once per rebuild
/// from `(document, InteractionMode)` and hands the same value to
/// each pass, so no two roles can disagree about what the user is
/// currently pointing at or previewing.
///
/// All four members borrow from the document, so a `FrameOverrides`
/// lives exactly as long as the `&MindMapDocument` it was derived
/// from. `Default` is "nothing selected, nothing previewed" — the
/// steady-state value and the one every pass fast-paths.
#[derive(Debug, Clone, Default)]
pub struct FrameOverrides<'a> {
    /// What the user has selected, plus the two inline editors'
    /// uncommitted buffers.
    pub selection: SceneSelectionContext<'a>,
    /// Color-picker hover on a line-mode edge.
    pub edge_color: Option<EdgeColorPreview<'a>>,
    /// The same hover fanned out to the portal pass, for when the
    /// previewed edge renders as a portal pair.
    pub portal_color: Option<PortalColorPreview<'a>>,
    /// Staged `border preview` / `section frame preview` edits.
    /// Read by the border pass, the section-frame pass, and the
    /// clip-AABB pass (a preview can change the border extent that
    /// connections clip against).
    pub border: Option<BorderPreview<'a>>,
}
