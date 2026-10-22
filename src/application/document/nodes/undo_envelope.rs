// SPDX-License-Identifier: MPL-2.0

//! The one snapshot → mutate → verdict → undo-push → auto-fit
//! envelope every node-scoped document setter runs inside.
//!
//! Three undo variants describe a per-node edit —
//! [`UndoAction::EditNodeStyle`], [`UndoAction::EditNodeText`],
//! and [`UndoAction::EditNodeAabb`] — and each one used to be
//! open-coded at a dozen call sites, together with the
//! `canvas_default` clone + `grow_one_node_to_fit_text` +
//! `grow_one_node_to_fit_border` tail. That fan-out shipped real
//! bugs twice (see the "Pre-fix" narrations on
//! [`UndoAction::EditNodeStyle`] and
//! [`MindMapDocument::set_node_size`]): a copy got fixed, its
//! siblings did not.
//!
//! Everything here exists so that a change to the envelope is a
//! one-site change. The pieces:
//!
//! - [`NodeEditTail`] — the post-commit passes a setter opts into.
//!   Auto-fit is a *policy* of the edit, not of the envelope: a
//!   color-only setter must not pay for a text re-measure, and a
//!   structural setter must additionally repair selection state.
//! - [`MindMapDocument::mutate_node_with_style_undo`] /
//!   [`MindMapDocument::mutate_node_with_text_undo`] — the two
//!   closure-verdict envelopes. The closure returns `Some(value)`
//!   to commit and `None` to declare a no-op.
//! - [`MindMapDocument::mutate_node_with_aabb_undo`] — the
//!   self-verdict envelope. `EditNodeAabb` carries exactly
//!   `(position, size)`, so the envelope compares those itself
//!   *after* the auto-fit tail; a framed node whose border-grow
//!   inflates past the requested size still no-ops on the second
//!   identical call.
//! - The two section-scoped wrappers, which narrow the closure to
//!   one [`MindSection`] and fold the index lookup in.
//!
//! **No-op semantics.** A `None` verdict restores `style`,
//! `color_schema`, `sections`, `position` and `size` from the
//! snapshot already taken for the undo entry, pushes nothing, and
//! leaves `dirty` alone. That is the whole point of the
//! closure-verdict shape: a caller can mutate first and decide
//! afterwards without reaching for the `undo_stack.pop()`
//! anti-pattern (which cannot restore `dirty` and breaks the
//! undo-LIFO invariant if any other entry slips in between the
//! push and the pop).
//!
//! **The restore set is the contract, and it is not the whole
//! node.** There is one per undo kind, because they reverse
//! different things: [`STYLE_RESTORE_SET`] for
//! [`UndoAction::EditNodeStyle`], and the strictly smaller
//! [`TEXT_RESTORE_SET`] for [`UndoAction::EditNodeText`], which
//! carries neither `before_style` nor `before_color_schema`. A
//! style closure may write `style`, `color_schema`, `sections`,
//! `position` and `size`; a text closure may write only the last
//! three. Everything else on [`MindNode`] is **out of bounds**:
//! `notes`, `folded`, `channel`, `trigger_bindings`,
//! `inline_mutations`, `inline_macros`, `min_zoom_to_render`,
//! `max_zoom_to_render`, `parent_id`, `id`, `layout`. A closure
//! that writes one of those persists it on a no-op verdict *and*
//! leaves it un-undoable on a commit — the `EditNodeStyle` /
//! `EditNodeText` arms of `undo()` will not put it back. No caller
//! does this today; a mutation that needs one of those fields
//! wants `UndoAction::CustomMutation` (which snapshots a whole
//! `target_scope` window) or a new variant, not this envelope.
//!
//! `color_schema` joined the style set when the per-node color
//! setters did: a themed node's fill / frame / text override lives
//! on its `ColorSchema`, because `style` is the tier the palette
//! shadows (`format/palettes.md`). Two homes for one node's colors
//! is the price of a theme that a per-node edit can except.
//!
//! **Missing node.** Every envelope resolves the id first and
//! yields the no-op result when it does not exist, so callers
//! never index a stale id — `CODE_CONVENTIONS.md` §9.

use baumhard::mindmap::model::{MindNode, MindSection};

use super::super::undo_action::UndoAction;
use super::super::{grow_one_node_to_fit_border, grow_one_node_to_fit_text};
use super::super::{MindMapDocument, SectionSel, SectionSpan, SelectionState};
use super::BorderPreviewTarget;

/// The fields [`UndoAction::EditNodeStyle`] restores, and so the
/// restore set of the two closure-verdict envelopes.
const STYLE_RESTORE_SET: &[&str] = &["style", "color_schema", "sections", "position", "size"];

/// The fields [`UndoAction::EditNodeText`] restores — a strict
/// subset of [`STYLE_RESTORE_SET`] that leaves out the two color
/// homes, because a text setter has no business writing either.
/// Held separately rather than sharing the style set: once
/// `color_schema` joined that set, a text closure quietly writing
/// a node's palette binding would have passed a guard whose whole
/// job is to catch a write the matching `undo()` arm cannot
/// reverse.
const TEXT_RESTORE_SET: &[&str] = &["sections", "position", "size"];

/// The fields [`UndoAction::EditNodeAabb`] restores — a strict
/// subset of [`STYLE_RESTORE_SET`], which is why
/// [`MindMapDocument::mutate_node_with_aabb_undo`] holds its
/// closures to a tighter contract than the other two.
const AABB_RESTORE_SET: &[&str] = &["position", "size"];

/// Debug-only fingerprint of every [`MindNode`] field *outside*
/// `restore_set` — see the "restore set is the contract" note in
/// the module header.
///
/// Derived by serializing the node and deleting the permitted
/// keys rather than by listing the forbidden ones, so a field
/// added to `MindNode` later is guarded from the moment it
/// exists instead of from whenever someone remembers to extend a
/// list here. Returns `()` in release builds, so the guard
/// compiles away entirely.
#[cfg(debug_assertions)]
fn fields_outside_restore_set(node: &MindNode, restore_set: &[&str]) -> String {
    let mut value = serde_json::to_value(node).unwrap_or(serde_json::Value::Null);
    if let Some(map) = value.as_object_mut() {
        for key in restore_set {
            map.remove(*key);
        }
    }
    value.to_string()
}

#[cfg(not(debug_assertions))]
fn fields_outside_restore_set(_node: &MindNode, _restore_set: &[&str]) {}

/// Fail loudly in debug when a closure wrote a field outside its
/// envelope's restore set. Such a write is silently wrong in two
/// directions at once — persisted on a no-op verdict, and left
/// un-undoable on a commit, because the matching `undo()` arm
/// never captured it. No-op in release.
#[cfg(debug_assertions)]
fn debug_assert_restore_set_respected(node: &MindNode, restore_set: &[&str], before: &str) {
    debug_assert_eq!(
        fields_outside_restore_set(node, restore_set),
        before,
        "a node-edit closure wrote outside its envelope's restore set ({restore_set:?}); \
         that write is neither undoable nor rolled back on a no-op verdict \
         — see the undo_envelope module header"
    );
}

#[cfg(not(debug_assertions))]
fn debug_assert_restore_set_respected(_node: &MindNode, _restore_set: &[&str], _before: &()) {}

/// Post-commit passes a node edit opts into. Named rather than
/// boolean because the four shapes are genuinely different
/// policies, and picking the wrong one is the class of bug this
/// module exists to prevent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::application::document) enum NodeEditTail {
    /// Nothing runs. For edits that cannot change either the
    /// measured text extent or the rendered border extent —
    /// colors, visibility of things that do not occupy space,
    /// per-section frame overrides (which live inside the node
    /// AABB).
    None,
    /// [`grow_one_node_to_fit_border`] only. The explicit-shrink
    /// path ([`MindMapDocument::fit_node_to_content`]) and the
    /// border-config setters: the text floor is either already
    /// satisfied or deliberately being shrunk past, but a framed
    /// node still needs room for its frame glyphs.
    Border,
    /// [`grow_one_node_to_fit_text`] then
    /// [`grow_one_node_to_fit_border`] — the standard tail for
    /// any edit that can change measured text extent (text, font
    /// family, font size, section geometry). Border runs second
    /// because a wider node may also need a wider frame.
    Grow,
    /// [`NodeEditTail::Grow`] plus
    /// [`cleanup_after_structural_mutation`]. Only for edits that
    /// change `sections.len()`, because only those can strand a
    /// selection or a border preview on a section index that no
    /// longer exists.
    GrowAndCleanup,
}

/// Which undo variant a closure-verdict envelope pushes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeUndoKind {
    /// [`UndoAction::EditNodeStyle`] — style and/or section
    /// formatting changed; the node's text content did not.
    Style,
    /// [`UndoAction::EditNodeText`] — the node's text content
    /// changed. Carries neither `before_style` nor
    /// `before_color_schema` because text setters write neither.
    Text,
}

impl NodeUndoKind {
    /// The fields this kind's undo variant restores, and so the
    /// contract its closure is held to.
    fn restore_set(self) -> &'static [&'static str] {
        match self {
            NodeUndoKind::Style => STYLE_RESTORE_SET,
            NodeUndoKind::Text => TEXT_RESTORE_SET,
        }
    }
}

impl MindMapDocument {
    /// Run `mutate` against `node_id` under a single
    /// [`UndoAction::EditNodeStyle`] entry, then `tail`.
    ///
    /// The closure returns `Some(value)` to commit — the value is
    /// handed back to the caller so structural mutators can
    /// report the index they inserted at — or `None` to declare a
    /// no-op, in which case the pre-mutation snapshot is restored
    /// and nothing is pushed. `None` is also the answer when
    /// `node_id` does not resolve, so `Option::is_some` is the
    /// honest "did anything change?" verdict for the many setters
    /// that return `bool`.
    ///
    /// Cost: two clones of `node.style` / `node.sections` worth of
    /// snapshot (one for the undo entry, reused for the no-op
    /// restore) plus whatever `tail` measures. Not safe to call
    /// per frame; drag callers gather a delta and commit once on
    /// release.
    pub(in crate::application::document) fn mutate_node_with_style_undo<F, R>(
        &mut self,
        node_id: &str,
        tail: NodeEditTail,
        mutate: F,
    ) -> Option<R>
    where
        F: FnOnce(&mut MindNode) -> Option<R>,
    {
        self.mutate_node_with_undo(node_id, NodeUndoKind::Style, tail, mutate)
    }

    /// Sibling of [`Self::mutate_node_with_style_undo`] that
    /// pushes [`UndoAction::EditNodeText`] instead. Same verdict,
    /// snapshot, restore, and tail semantics; the variant differs
    /// only in that it carries no `before_style`, because the
    /// round-trip contract for a text edit is "text and its runs
    /// come back, the style was never touched".
    pub(in crate::application::document) fn mutate_node_with_text_undo<F, R>(
        &mut self,
        node_id: &str,
        tail: NodeEditTail,
        mutate: F,
    ) -> Option<R>
    where
        F: FnOnce(&mut MindNode) -> Option<R>,
    {
        self.mutate_node_with_undo(node_id, NodeUndoKind::Text, tail, mutate)
    }

    /// Shared body of the two closure-verdict envelopes.
    fn mutate_node_with_undo<F, R>(
        &mut self,
        node_id: &str,
        kind: NodeUndoKind,
        tail: NodeEditTail,
        mutate: F,
    ) -> Option<R>
    where
        F: FnOnce(&mut MindNode) -> Option<R>,
    {
        let node = self.mindmap.nodes.get(node_id)?;
        let before_style = node.style.clone();
        let before_color_schema = node.color_schema.clone();
        let before_sections = node.sections.clone();
        let before_position = node.position;
        let before_size = node.size;
        let before_selection = self.selection.clone();
        let canvas_default = self.mindmap.canvas.default_border.clone();

        let restore_set = kind.restore_set();
        let before_untouched = fields_outside_restore_set(node, restore_set);

        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("id resolved immutably one statement ago");
        let committed = match mutate(node) {
            Some(value) => value,
            None => {
                // Restore precisely the fields the undo entry
                // would have restored. Anything outside that set
                // was never undoable through this envelope, so
                // restoring it would be a promise the commit path
                // does not keep.
                node.style = before_style;
                node.color_schema = before_color_schema;
                node.sections = before_sections;
                node.position = before_position;
                node.size = before_size;
                debug_assert_restore_set_respected(node, restore_set, &before_untouched);
                return None;
            }
        };
        debug_assert_restore_set_respected(node, restore_set, &before_untouched);

        self.undo_stack.push(match kind {
            NodeUndoKind::Style => UndoAction::EditNodeStyle {
                node_id: node_id.to_string(),
                before_style,
                before_color_schema,
                before_sections,
                before_position,
                before_size,
                before_selection,
            },
            NodeUndoKind::Text => UndoAction::EditNodeText {
                node_id: node_id.to_string(),
                before_sections,
                before_position,
                before_size,
                before_selection,
            },
        });
        self.dirty = true;
        self.run_node_edit_tail(node_id, tail, canvas_default.as_ref());
        Some(committed)
    }

    /// Run `mutate` against `node_id` under a single
    /// [`UndoAction::EditNodeAabb`] entry, then `tail`, then
    /// decide whether anything actually changed.
    ///
    /// Unlike the style / text envelopes there is no closure
    /// verdict: `EditNodeAabb` carries exactly `(position, size)`,
    /// so the envelope compares those two fields itself — and it
    /// does so **after** the tail, because a framed node's
    /// border-grow can inflate `size` past the value the caller
    /// asked for. Gating before the tail would make every call
    /// after the first on a framed node look like a change and
    /// stack undo entries (that was a real shipped bug; see the
    /// idempotency note on [`MindMapDocument::set_node_size`]).
    ///
    /// The closure must confine itself to `position` / `size`:
    /// anything else it writes is neither compared nor undoable
    /// here.
    ///
    /// Returns `true` when the entry was pushed, `false` on a
    /// missing node or an AABB that came out where it started.
    pub(in crate::application::document) fn mutate_node_with_aabb_undo<F>(
        &mut self,
        node_id: &str,
        tail: NodeEditTail,
        mutate: F,
    ) -> bool
    where
        F: FnOnce(&mut MindNode),
    {
        let Some(node) = self.mindmap.nodes.get(node_id) else {
            return false;
        };
        let before_position = node.position;
        let before_size = node.size;
        let before_untouched = fields_outside_restore_set(node, AABB_RESTORE_SET);
        let canvas_default = self.mindmap.canvas.default_border.clone();

        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("id resolved immutably one statement ago");
        mutate(node);
        self.run_node_edit_tail(node_id, tail, canvas_default.as_ref());

        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("id resolved immutably above");
        debug_assert_restore_set_respected(node, AABB_RESTORE_SET, &before_untouched);
        // Position is untouched by the auto-fit passes, so the
        // comparison against `before_position` is exact; `size` is
        // compared post-grow for the idempotency reason above.
        let same_position = node.position.x == before_position.x && node.position.y == before_position.y;
        if same_position && node.size == before_size {
            return false;
        }
        self.undo_stack.push(UndoAction::EditNodeAabb {
            node_id: node_id.to_string(),
            before_position,
            before_size,
        });
        self.dirty = true;
        true
    }

    /// The auto-fit (and, for structural edits, repair) passes
    /// that follow a committed node edit. Split out so all three
    /// envelopes share one definition of what each
    /// [`NodeEditTail`] means.
    fn run_node_edit_tail(
        &mut self,
        node_id: &str,
        tail: NodeEditTail,
        canvas_default: Option<&baumhard::mindmap::model::GlyphBorderConfig>,
    ) {
        if tail == NodeEditTail::None {
            return;
        }
        let Some(node) = self.mindmap.nodes.get_mut(node_id) else {
            return;
        };
        match tail {
            // Already returned above; spelled as a no-op rather
            // than `unreachable!` so that if the early return and
            // this arm ever drift apart the result is a skipped
            // pass, not a panic on a document-mutation path
            // (`CODE_CONVENTIONS.md` §9).
            NodeEditTail::None => {}
            NodeEditTail::Border => grow_one_node_to_fit_border(node, canvas_default),
            NodeEditTail::Grow | NodeEditTail::GrowAndCleanup => {
                grow_one_node_to_fit_text(node);
                grow_one_node_to_fit_border(node, canvas_default);
            }
        }
        if tail == NodeEditTail::GrowAndCleanup {
            cleanup_after_structural_mutation(self, node_id);
        }
    }

    /// Section-scoped [`Self::mutate_node_with_style_undo`]: the
    /// closure sees one [`MindSection`] and returns its own
    /// change verdict as a `bool`. A `false` verdict backs the
    /// section out from the snapshot, so a caller may mutate
    /// speculatively and decide afterwards.
    ///
    /// A `section_idx` past the end is the same no-op as a
    /// missing node — the lookup is folded in here so no caller
    /// has to index `sections` and risk a panic in an interactive
    /// path.
    pub(in crate::application::document) fn mutate_section_with_style_undo<F>(
        &mut self,
        node_id: &str,
        section_idx: usize,
        tail: NodeEditTail,
        mutate: F,
    ) -> bool
    where
        F: FnOnce(&mut MindSection) -> bool,
    {
        self.mutate_node_with_style_undo(node_id, tail, |node| section_verdict(node, section_idx, mutate))
            .is_some()
    }

    /// Section-scoped [`Self::mutate_node_with_text_undo`]. Same
    /// shape as [`Self::mutate_section_with_style_undo`]; use this
    /// one whenever `section.text` itself is being rewritten so
    /// the undo entry describes what the user actually did.
    pub(in crate::application::document) fn mutate_section_with_text_undo<F>(
        &mut self,
        node_id: &str,
        section_idx: usize,
        tail: NodeEditTail,
        mutate: F,
    ) -> bool
    where
        F: FnOnce(&mut MindSection) -> bool,
    {
        self.mutate_node_with_text_undo(node_id, tail, |node| section_verdict(node, section_idx, mutate))
            .is_some()
    }
}

/// Adapt a section-scoped `bool`-returning closure to the
/// node-scoped `Option` verdict the envelopes speak.
fn section_verdict<F>(node: &mut MindNode, section_idx: usize, mutate: F) -> Option<()>
where
    F: FnOnce(&mut MindSection) -> bool,
{
    let section = node.sections.get_mut(section_idx)?;
    if mutate(section) {
        Some(())
    } else {
        None
    }
}

/// In-doc repair that runs after a structural section mutation
/// (add / delete / split) on `node_id`. Cancels any active border
/// preview that targeted the affected sections (its index
/// reference is now potentially stale), and clamps the live
/// `selection` so a `Section` / `SectionRange` / `MultiSection`
/// selection that pointed past the new section count gets
/// retargeted to a valid index (or demoted to a whole-node
/// selection when the original section is gone).
///
/// **Scope**: covers the doc-side state (`border_preview` and
/// `selection`). App-side state — `TextEditState`,
/// `DragState::Throttled(SectionResize)`, `SingleLineEditor` —
/// lives in `InitState` and is reachable only from the app
/// layer; those concerns are handled at the console verb
/// dispatch site (see `console::commands::section::mod.rs`).
fn cleanup_after_structural_mutation(doc: &mut MindMapDocument, node_id: &str) {
    let new_count = doc
        .mindmap
        .nodes
        .get(node_id)
        .map(|n| n.sections.len())
        .unwrap_or(0);

    // Cancel any active border preview whose Section / Sections
    // target lands on this node — a structural mutation
    // invalidates the preview's idx reference. The drift
    // mechanism in `border_preview_covers_live_selection` only
    // catches selection-vs-target drift, not target-shift after
    // a structural mutation.
    let preview_targets_this_node = doc
        .border_preview
        .as_ref()
        .map(|p| match &p.target {
            BorderPreviewTarget::Sections(pairs) => pairs.iter().any(|(id, _)| id == node_id),
            BorderPreviewTarget::Nodes(ids) => ids.iter().any(|id| id == node_id),
            // Canvas-default previews are orthogonal to per-section
            // structural changes — they don't reference the node.
            _ => false,
        })
        .unwrap_or(false);
    if preview_targets_this_node {
        doc.cancel_border_preview();
    }

    // Clamp the selection's section_idx to the new count. A
    // `Section` selection past the end demotes to `Single(node)`
    // (the natural "section is gone" lift). `SectionRange` clamps
    // both ends of its **section span**; if the anchor section is
    // gone, demote. The grapheme range is left as authored — it
    // addresses text inside the surviving anchor section, and the
    // range-aware setters clamp against the live grapheme count on
    // every use, which a structural section mutation does not
    // change for sections it did not touch. `MultiSection` filters
    // out the dead pairs; if none survive, demote.
    match &doc.selection {
        SelectionState::Section(s) if s.node_id == node_id && s.section_idx >= new_count => {
            doc.selection = SelectionState::Single(node_id.to_string());
        }
        SelectionState::SectionRange {
            sel,
            section_span,
            grapheme_range,
        } if sel.node_id == node_id => {
            let max_idx = new_count.saturating_sub(1);
            let lo = section_span.lo().min(max_idx);
            let hi = section_span.hi().min(max_idx);
            if new_count == 0 {
                doc.selection = SelectionState::Single(node_id.to_string());
            } else if sel.section_idx >= new_count {
                // Anchor section gone — demote to a single
                // surviving section selection at the closest
                // remaining idx. The grapheme range dies with its
                // anchor; a sub-range of deleted text is not a
                // sub-range of whatever now sits at `lo`.
                doc.selection = SelectionState::Section(SectionSel {
                    node_id: node_id.to_string(),
                    section_idx: lo,
                });
            } else {
                doc.selection = SelectionState::SectionRange {
                    sel: sel.clone(),
                    section_span: SectionSpan::new(lo, hi),
                    grapheme_range: *grapheme_range,
                };
            }
        }
        SelectionState::MultiSection(sels) => {
            let surviving: Vec<SectionSel> = sels
                .iter()
                .filter(|s| s.node_id != node_id || s.section_idx < new_count)
                .cloned()
                .collect();
            doc.selection = match surviving.len() {
                0 => SelectionState::Single(node_id.to_string()),
                1 => SelectionState::Section(surviving.into_iter().next().expect("len==1")),
                _ => SelectionState::MultiSection(surviving),
            };
        }
        _ => {} // Single / Multi / Edge / Portal / None — no idx to clamp.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::{
        first_testament_node_id as first_node_id, load_test_doc as fixture_doc,
    };

    /// A `None` verdict restores every field the undo entry
    /// would have restored — style, sections, position, size —
    /// and pushes nothing. The contract that lets a caller
    /// mutate speculatively.
    #[test]
    fn test_style_envelope_none_verdict_restores_and_pushes_nothing() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        let before = doc.mindmap.nodes.get(&id).expect("node").clone();
        doc.undo_stack.clear();
        doc.dirty = false;

        let out: Option<()> = doc.mutate_node_with_style_undo(&id, NodeEditTail::Grow, |node| {
            node.style.background_color = "#123456".to_string();
            node.sections[0].text = "scribble".to_string();
            node.size.width += 500.0;
            None
        });

        assert!(out.is_none(), "None verdict must surface as None");
        let after = doc.mindmap.nodes.get(&id).expect("node");
        assert_eq!(after.style.background_color, before.style.background_color);
        assert_eq!(after.sections[0].text, before.sections[0].text);
        assert_eq!(after.size.width, before.size.width);
        assert!(doc.undo_stack.is_empty(), "no-op must not push undo");
        assert!(!doc.dirty, "no-op must not dirty the document");
    }

    /// A `Some` verdict pushes one `EditNodeStyle`, dirties the
    /// document, hands the closure's value back, and runs the
    /// requested tail.
    #[test]
    fn test_style_envelope_some_verdict_commits_and_returns_value() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        doc.undo_stack.clear();
        doc.dirty = false;

        let out = doc.mutate_node_with_style_undo(&id, NodeEditTail::None, |node| {
            node.style.background_color = "#123456".to_string();
            Some(7_usize)
        });

        assert_eq!(out, Some(7));
        assert_eq!(
            doc.mindmap.nodes.get(&id).expect("node").style.background_color,
            "#123456"
        );
        assert!(doc.dirty);
        assert_eq!(doc.undo_stack.len(), 1);
        assert!(matches!(doc.undo_stack[0], UndoAction::EditNodeStyle { .. }));
    }

    /// The text envelope pushes `EditNodeText`, not
    /// `EditNodeStyle` — the two round-trip contracts are
    /// distinct and the variant is what tells `undo()` which one
    /// applies.
    #[test]
    fn test_text_envelope_pushes_edit_node_text_variant() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        doc.undo_stack.clear();

        let out = doc.mutate_node_with_text_undo(&id, NodeEditTail::Grow, |node| {
            node.sections[0].text = "rewritten".to_string();
            node.sections[0].text_runs.clear();
            Some(())
        });

        assert!(out.is_some());
        assert_eq!(doc.undo_stack.len(), 1);
        assert!(matches!(doc.undo_stack[0], UndoAction::EditNodeText { .. }));
        assert!(doc.undo(), "undo must succeed");
        assert_ne!(
            doc.mindmap.nodes.get(&id).expect("node").sections[0].text,
            "rewritten"
        );
    }

    /// A missing node id is the same no-op as a `None` verdict —
    /// no panic, no push, `None` out.
    #[test]
    fn test_envelopes_no_op_on_missing_node() {
        let mut doc = fixture_doc();
        doc.undo_stack.clear();
        doc.dirty = false;
        let out: Option<()> =
            doc.mutate_node_with_style_undo("no-such-node", NodeEditTail::Grow, |_| Some(()));
        assert!(out.is_none());
        assert!(!doc.mutate_node_with_aabb_undo("no-such-node", NodeEditTail::Grow, |_| {}));
        assert!(doc.undo_stack.is_empty());
        assert!(!doc.dirty);
    }

    /// A `section_idx` past the end is a no-op, not a panic —
    /// the index lookup lives inside the envelope precisely so
    /// no caller can get this wrong.
    #[test]
    fn test_section_envelope_no_op_on_out_of_range_index() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        doc.undo_stack.clear();
        doc.dirty = false;
        let changed = doc.mutate_section_with_style_undo(&id, 9999, NodeEditTail::Grow, |s| {
            s.text = "unreachable".to_string();
            true
        });
        assert!(!changed);
        assert!(doc.undo_stack.is_empty());
        assert!(!doc.dirty);
    }

    /// The AABB envelope's verdict is computed after the tail:
    /// re-running the same write is a no-op even on a framed
    /// node whose border-grow inflated the size past what was
    /// asked for.
    #[test]
    fn test_aabb_envelope_is_idempotent_across_the_grow_tail() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        doc.mindmap.nodes.get_mut(&id).expect("node").style.show_frame = true;
        let target = baumhard::mindmap::model::Size {
            width: 40.0,
            height: 20.0,
        };
        assert!(doc.mutate_node_with_aabb_undo(&id, NodeEditTail::Grow, |n| n.size = target));
        doc.undo_stack.clear();
        doc.dirty = false;
        assert!(
            !doc.mutate_node_with_aabb_undo(&id, NodeEditTail::Grow, |n| n.size = target),
            "second identical write must no-op after the grow tail"
        );
        assert!(doc.undo_stack.is_empty());
        assert!(!doc.dirty);
    }

    /// `NodeEditTail::None` really does skip the auto-fit — a
    /// color-only setter must not pay for a text re-measure,
    /// and must not silently inflate a node the user shrank.
    #[test]
    fn test_tail_none_leaves_size_untouched() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        {
            let node = doc.mindmap.nodes.get_mut(&id).expect("node");
            node.style.show_frame = false;
            node.size = baumhard::mindmap::model::Size {
                width: 1.0,
                height: 1.0,
            };
        }
        let out = doc.mutate_node_with_style_undo(&id, NodeEditTail::None, |node| {
            node.style.background_color = "#001122".to_string();
            Some(())
        });
        assert!(out.is_some());
        let size = doc.mindmap.nodes.get(&id).expect("node").size;
        assert_eq!(size.width, 1.0, "tail None must not grow the node");
        assert_eq!(size.height, 1.0);
    }
}
