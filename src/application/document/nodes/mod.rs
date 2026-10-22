// SPDX-License-Identifier: MPL-2.0

//! Per-node and per-section-geometry setters and node-style
//! helpers. Section text / color / font / runs / payload
//! setters live in `section_text.rs`. Every setter here routes
//! through the shared envelope in `undo_envelope.rs`, which owns
//! the snapshot → verdict → undo-push → auto-fit sequence; a
//! setter's whole job is to validate its input, express the
//! field write as a closure, and pick a [`NodeEditTail`].

use baumhard::mindmap::model::{validate, TextRun};

use super::compute_one_node_text_floor;
use super::defaults::default_text_run;
use super::undo_action::UndoAction;
use super::MindMapDocument;

mod border;
mod option_edit;
mod section_structure;
mod section_text;
mod undo_envelope;

pub(in crate::application::document) use undo_envelope::NodeEditTail;

pub use border::{BorderConfigEdits, BorderEditOutcome, BorderPreview, BorderPreviewTarget, BorderSide};
// Test-only re-export of the slot helper. Production code routes
// through the four committing setters; the parity test in
// `tests_nodes.rs` reaches for the helper directly to exercise
// it against `apply_view_to_slot`.
#[cfg(test)]
pub(in crate::application) use border::apply_glyph_border_edits_to_slot;
// Test-only: the shipped callers are inside `border` itself.
#[cfg(test)]
pub(in crate::application::document) use border::merge_outcome;
pub use option_edit::OptionEdit;
pub(in crate::application::document) use section_text::{
    clamp_runs_to_text, section_runs_all_match, write_every_section_run,
};

/// Which of a node's three color channels a per-node write
/// targets.
///
/// Named rather than written out three times: *where* the value
/// lands — the node's own `color_schema.overrides` on a themed
/// node, `node.style` otherwise — is one rule, and the three
/// channels differ only in which field it picks. Three copies of
/// that rule is how "change this node's fill" came to be broken on
/// themed nodes while "change its text color" happened to survive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeColorChannel {
    Background,
    Frame,
    Text,
}

impl NodeColorChannel {
    /// The color this channel has on a node nothing has said
    /// anything about — [`default_orphan_node`]'s `NodeStyle`.
    ///
    /// Only the **unthemed** half of a clear reaches this. A
    /// themed node clears to its palette group, which is the
    /// natural default it actually has; an unthemed node has no
    /// tier underneath `style`, so the floor has to be named.
    ///
    /// [`default_orphan_node`]: super::defaults::default_orphan_node
    fn unthemed_default(self) -> &'static str {
        use super::defaults::{
            DEFAULT_NODE_BACKGROUND_COLOR, DEFAULT_NODE_FRAME_COLOR, DEFAULT_NODE_TEXT_COLOR,
        };
        match self {
            NodeColorChannel::Background => DEFAULT_NODE_BACKGROUND_COLOR,
            NodeColorChannel::Frame => DEFAULT_NODE_FRAME_COLOR,
            NodeColorChannel::Text => DEFAULT_NODE_TEXT_COLOR,
        }
    }

    /// Whether the empty string is a *value* on this channel.
    ///
    /// Only on `background`, where it spells "no fill, let the
    /// canvas show through" and every reader passes it along. On
    /// `frame` and `text` there is no transparent spelling — a
    /// frame is switched off with `show_frame` and text has no off
    /// — so the readers skip an empty value and take the next
    /// tier (`MindMap::node_frame_theme_tier`,
    /// `MindMap::node_text_color`). Writing one there would report
    /// success and paint nothing new, which is the whole defect
    /// this branch exists to close.
    fn empty_is_a_color(self) -> bool {
        self == NodeColorChannel::Background
    }
}

/// What a per-node color write is asking for, once the empty
/// string has been read against the channel it landed on.
///
/// The two arms are genuinely different requests, and the tier
/// the node has decides how each is spelled — which is why this
/// resolution happens in one place rather than at the three
/// public setters. See [`MindMapDocument::set_node_color_channel`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeColorWrite<'a> {
    /// Paint this channel this color.
    Set(&'a str),
    /// This node has no color of its own on this channel: take
    /// whatever it would have had. On a themed node that is the
    /// palette group, reached by clearing the override slot; on an
    /// unthemed node there is nothing below `style`, so it is
    /// [`NodeColorChannel::unthemed_default`].
    Clear,
}

/// Snapshot of a `MindSection`'s user-facing fields, used by the
/// structured-clipboard path (`ClipboardContent::Section` carries
/// it, the in-process buffer in `application/clipboard.rs` stashes
/// it, `apply_section_payload` writes it back). Decoupled from the
/// trait layer so callers can build payloads without depending on
/// `console::traits::outcome`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SectionPayload {
    pub text_runs: Vec<TextRun>,
    pub offset: baumhard::mindmap::model::Position,
    pub size: Option<baumhard::mindmap::model::Size>,
    pub channel: Option<usize>,
    pub trigger_bindings: Vec<baumhard::mindmap::custom_mutation::TriggerBinding>,
}

impl SectionPayload {
    /// Snapshot a `MindSection` into a payload (deep-clone each
    /// field). Cheap — every contained type is `Clone`.
    pub fn from_section(section: &baumhard::mindmap::model::MindSection) -> Self {
        Self {
            text_runs: section.text_runs.clone(),
            offset: section.offset,
            size: section.size,
            channel: section.channel,
            trigger_bindings: section.trigger_bindings.clone(),
        }
    }
}

impl MindMapDocument {
    /// Set one section's `offset` (relative to its owning node's
    /// `position`) under a single `EditNodeStyle` undo entry.
    /// Drag callers must NOT invoke this per-frame; gather delta
    /// in a gesture-state shape and call once on release.
    pub fn set_section_offset(
        &mut self,
        node_id: &str,
        section_idx: usize,
        x: f64,
        y: f64,
    ) -> Result<bool, String> {
        // Validate before mutating so callers that pre-flight
        // (e.g. `execute_move_fan_out_multisection`'s atomic
        // parse-then-dispatch) can reuse the same predicate via
        // [`Self::validate_section_offset_change`] and get
        // identical error messages.
        self.validate_section_offset_change(node_id, section_idx, x, y)?;
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Ok(false),
        };
        let Some(section) = node.sections.get(section_idx) else {
            return Ok(false);
        };
        let new_offset = baumhard::mindmap::model::Position { x, y };
        if section.offset == new_offset {
            return Ok(false);
        }
        // `NodeEditTail::Grow`: moving a `None`-sized section can
        // shift its measured-text floor contribution beyond the
        // current `node.size`, leaving the node under its floor
        // for the next unrelated edit.
        Ok(
            self.mutate_section_with_style_undo(node_id, section_idx, NodeEditTail::Grow, |s| {
                s.offset.x = x;
                s.offset.y = y;
                true
            }),
        )
    }

    /// Pre-validate `set_section_offset(node, idx, x, y)` without
    /// mutating. Returns the same `Err(msg)` the setter would
    /// produce, or `Ok(())` if the call would succeed
    /// (including the "section not found → silent Ok(false)"
    /// case — the validator only rejects bad geometry, not
    /// stale ids).
    ///
    /// Used by `execute_move_fan_out_multisection` to pre-flight
    /// every (node, section) pair before any mutation, matching
    /// the parse-then-dispatch shape of `section/frame.rs::apply_edits`
    /// so a partial fan-out can never land.
    pub fn validate_section_offset_change(
        &self,
        node_id: &str,
        section_idx: usize,
        x: f64,
        y: f64,
    ) -> Result<(), String> {
        let Some(node) = self.mindmap.nodes.get(node_id) else {
            return Ok(());
        };
        let Some(section) = node.sections.get(section_idx) else {
            return Ok(());
        };
        let new_offset = baumhard::mindmap::model::Position { x, y };
        validate::section_candidate_aabb(node.size, section_idx, new_offset, section.size)
    }

    /// Set one section's `size`. `None` means fill-parent;
    /// `Some(Size)` pins an explicit AABB. Same verify-mirroring
    /// validation discipline as [`Self::set_section_offset`]; same
    /// no-per-frame contract for drag callers.
    pub fn set_section_size(
        &mut self,
        node_id: &str,
        section_idx: usize,
        size: Option<baumhard::mindmap::model::Size>,
    ) -> Result<bool, String> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Ok(false),
        };
        let Some(section) = node.sections.get(section_idx) else {
            return Ok(false);
        };
        validate::section_candidate_aabb(node.size, section_idx, section.offset, size)?;
        if section.size == size {
            return Ok(false);
        }
        Ok(
            self.mutate_section_with_style_undo(node_id, section_idx, NodeEditTail::Grow, |s| {
                s.size = size;
                true
            }),
        )
    }

    /// Atomically set one section's `(offset, size)` under a
    /// single `EditNodeStyle` undo entry. Validates the
    /// **post-mutation** AABB so a gesture that shifts offset and
    /// grows size in the same frame doesn't fail on the
    /// intermediate state.
    // Native-driver-only, with `set_node_aabb`: both are written by
    // the native resize state machine.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn set_section_aabb(
        &mut self,
        node_id: &str,
        section_idx: usize,
        new_offset: baumhard::mindmap::model::Position,
        new_size: baumhard::mindmap::model::Size,
    ) -> Result<bool, String> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Ok(false),
        };
        let Some(section) = node.sections.get(section_idx) else {
            return Ok(false);
        };
        validate::section_candidate_aabb(node.size, section_idx, new_offset, Some(new_size))?;
        if section.offset == new_offset && section.size == Some(new_size) {
            return Ok(false);
        }
        Ok(
            self.mutate_section_with_style_undo(node_id, section_idx, NodeEditTail::Grow, |s| {
                s.offset = new_offset;
                s.size = Some(new_size);
                true
            }),
        )
    }

    /// Set a node's `size` under a single `EditNodeAabb` undo
    /// entry. Validates finite + strictly positive components and
    /// rejects astronomical typos against `MAX_NODE_AXIS`. Position
    /// stays unchanged. Used by the `node resize <w> <h>` console
    /// verb.
    ///
    /// Idempotent: [`Self::mutate_node_with_aabb_undo`] gates on
    /// the *post-grow* size, so a framed node whose border-grow
    /// inflates past `new_size` still no-ops on repeated calls.
    ///
    /// Drag callers must NOT invoke this per-frame; gather delta
    /// in a gesture-state shape and call once on release via
    /// [`Self::set_node_aabb`] which atomically writes both
    /// position and size.
    pub fn set_node_size(
        &mut self,
        node_id: &str,
        new_size: baumhard::mindmap::model::Size,
    ) -> Result<bool, String> {
        validate::node_size(new_size)?;
        Ok(self.mutate_node_with_aabb_undo(node_id, NodeEditTail::Grow, |n| n.size = new_size))
    }

    /// Set a node's `(position, size)` atomically under a single
    /// `EditNodeAabb` undo entry. Used by the node-resize gesture's
    /// release-commit — corner / edge handles whose `axis_factors`
    /// shrink size by the same delta they shift offset by need
    /// the AABB written in lockstep so the undo stack carries one
    /// pre-edit pair, not two interleaved entries.
    ///
    /// Same post-grow no-op-gate discipline as
    /// [`Self::set_node_size`] — see there for the framed-node
    /// idempotency rationale.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn set_node_aabb(
        &mut self,
        node_id: &str,
        new_position: baumhard::mindmap::model::Position,
        new_size: baumhard::mindmap::model::Size,
    ) -> Result<bool, String> {
        validate_node_position(new_position)?;
        validate::node_size(new_size)?;
        Ok(self.mutate_node_with_aabb_undo(node_id, NodeEditTail::Grow, |n| {
            n.position = new_position;
            n.size = new_size;
        }))
    }

    /// Shrink (or grow) a node's `size` to its measured-text
    /// floor — the explicit-shrink path the ambient `grow_*`
    /// passes can't take. Border-bearing nodes are rounded up
    /// from the text floor by `grow_one_node_to_fit_border` so
    /// the rendered frame has room. Pushes one `EditNodeAabb`
    /// undo entry; idempotent (no entry pushed when already at
    /// the post-border-grow target). See `set_node_size` for
    /// the floor-rejection counterpart.
    ///
    /// Re-measures every section under per-section
    /// `FONT_SYSTEM` write-guard acquires — same cost shape as
    /// `grow_one_node_to_fit_text`. Drag callers must NOT
    /// invoke this per-frame.
    pub fn fit_node_to_content(&mut self, node_id: &str) -> Result<bool, String> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Ok(false),
        };
        let (floor_w, floor_h) = compute_one_node_text_floor(node);
        // `f64::NAN <= 0.0` is false, so the finite-check is
        // load-bearing — NaN from a bad-font-measure path would
        // otherwise slip through the simple `<= 0.0` gate.
        if !floor_w.is_finite() || !floor_h.is_finite() || floor_w <= 0.0 || floor_h <= 0.0 {
            return Err(format!(
                "node '{}' has no measurable text; fit-to-content has no target floor",
                node_id
            ));
        }
        let candidate = baumhard::mindmap::model::Size {
            width: floor_w,
            height: floor_h,
        };
        // Route the candidate through the same validation +
        // typo guard the sibling node-size setters use, so a
        // pinned `section.size` of e.g. 5_000_000 (which
        // propagates through the floor) can't bypass the
        // absolute ceiling. Cheap arithmetic; defends future
        // regressions in `compute_one_node_text_floor`.
        validate::node_size(candidate)?;
        // `NodeEditTail::Border`, not `Grow`: this is the shrink
        // path, and the text-grow pass would max-wins straight
        // back up to the floor we just wrote. The border floor
        // still runs — the rendered frame needs room.
        Ok(self.mutate_node_with_aabb_undo(node_id, NodeEditTail::Border, |n| n.size = candidate))
    }

    /// Replace the text of a node's **first** section, collapsing
    /// that section to a single run spanning the new text.
    ///
    /// Pre-section-refactor this setter wrote `node.text`; post-
    /// refactor it writes the first section's text. Multi-section
    /// nodes only have their first section edited here — the
    /// per-section surface is [`Self::set_section_text`].
    ///
    /// `NodeEditTail::Grow`: longer text on the same face
    /// overflows the right edge, and the monotonic floor only
    /// applies if we re-measure on the write.
    pub fn set_node_text(&mut self, node_id: &str, new_text: String) -> bool {
        self.mutate_node_with_text_undo(node_id, NodeEditTail::Grow, move |node| {
            let section = node.sections.first_mut()?;
            if section.text == new_text {
                return None;
            }
            // Inherit formatting from the first original run on
            // the section, or fall back to the authoring defaults.
            let template = section
                .text_runs
                .first()
                .cloned()
                .unwrap_or_else(|| default_text_run(0));
            // Empty text yields an empty runs vec: a
            // `TextRun { start: 0, end: 0 }` violates the
            // `text_run_ops` `start < end` invariant and panics
            // in debug builds on the next slice / splice /
            // find_run_containing call. Same guard as
            // [`Self::set_section_text`]; this setter used to
            // lack it, which is exactly the copy-drift the
            // shared envelope exists to end.
            let count = baumhard::util::grapheme_chad::count_grapheme_clusters(&new_text);
            section.text_runs = if count == 0 {
                Vec::new()
            } else {
                vec![baumhard::mindmap::model::TextRun {
                    start: 0,
                    end: count,
                    ..template
                }]
            };
            section.text = new_text;
            Some(())
        })
        .is_some()
    }

    /// Set this node's **fill** color, or clear it with `None`.
    /// Returns `true` if the value actually changed. Pushes one
    /// `UndoAction::EditNodeStyle` entry so undo restores the
    /// `NodeStyle`, the `ColorSchema` *and* the `text_runs`
    /// (unchanged for this setter, but the variant always carries
    /// all three so the undo arm has a single shape).
    ///
    /// **Where the value lands depends on whether the node is
    /// themed**, because it has to land where the read path looks
    /// (`MindMap::node_background_color`). A themed node's fill
    /// comes from its palette group, which shadows
    /// `style.background_color` entirely, so writing `style` there
    /// would report success and change nothing on screen; the
    /// write goes to the node's own `color_schema.overrides`
    /// instead. An unthemed node has no such tier and takes the
    /// `style` write directly. See `format/palettes.md`.
    ///
    /// `None` means "this node has no fill of its own": a themed
    /// node goes back to its palette group, an unthemed one to
    /// [`DEFAULT_NODE_BACKGROUND_COLOR`]. `Some("")` is *not* the
    /// same request — on this channel the empty string is a color,
    /// the transparent one, and it is written through.
    ///
    /// [`DEFAULT_NODE_BACKGROUND_COLOR`]: super::defaults::DEFAULT_NODE_BACKGROUND_COLOR
    ///
    /// No-op on missing node id, matching the `EditEdge` pattern.
    pub fn set_node_bg_color(&mut self, node_id: &str, color: Option<&str>) -> bool {
        self.set_node_color_channel(node_id, NodeColorChannel::Background, color)
    }

    /// Set this node's **frame** (border) color, or clear it with
    /// `None`. Returns `true` on change. Lands in the same two
    /// places for the same reason as [`Self::set_node_bg_color`].
    ///
    /// This is the cascade *base* the border resolver sits on: a
    /// node carrying an explicit `style.border.color` keeps
    /// painting that, and this setter does not touch it — the
    /// `border color=` verb is the one that does.
    ///
    /// `None` clears: back to the palette group on a themed node,
    /// to [`DEFAULT_NODE_FRAME_COLOR`] on an unthemed one. Unlike
    /// the fill channel, `Some("")` clears too — a frame has no
    /// transparent spelling (that is `show_frame`), the readers
    /// skip an empty frame, and writing one would report success
    /// and paint nothing new.
    ///
    /// [`DEFAULT_NODE_FRAME_COLOR`]: super::defaults::DEFAULT_NODE_FRAME_COLOR
    pub fn set_node_border_color(&mut self, node_id: &str, color: Option<&str>) -> bool {
        self.set_node_color_channel(node_id, NodeColorChannel::Frame, color)
    }

    /// Set the *default* text color on a node, and recolor every
    /// [`TextRun`] that was
    /// following that default. `None` clears instead of setting.
    ///
    /// The default itself lands in the same two places as
    /// [`Self::set_node_bg_color`] — the node's own overrides when
    /// it is themed, `style.text_color` when it is not — and
    /// clears the same way: back to the palette group when themed,
    /// to [`DEFAULT_NODE_TEXT_COLOR`] when not. `Some("")` clears
    /// as well, for the same reason it does on the frame channel.
    ///
    /// [`DEFAULT_NODE_TEXT_COLOR`]: super::defaults::DEFAULT_NODE_TEXT_COLOR
    ///
    /// Runs are treated in three groups, which is the whole
    /// content of "following the default":
    ///
    /// - A run with an **empty** `color` defers to the node's text
    ///   color by construction. It needs no rewrite and gets none;
    ///   it follows the new value for free, and rewriting it would
    ///   opt those graphemes out of the cascade for good.
    /// - A run whose `color` byte-matches the **effective** pre-edit
    ///   text color is a baked copy of the old default — the shape
    ///   `maptool convert --legacy` leaves behind — and is rewritten
    ///   so the node recolors as a whole.
    /// - Every other run is a deliberate per-span override and is
    ///   left alone.
    ///
    /// The match is byte-exact, deliberately: if the effective
    /// default is `"#FFFFFF"` but a run carries `"#ffffff"`, the
    /// run is *not* considered default-following and keeps its
    /// lowercase override. Colors are strings in the model and
    /// comparisons are literal, matching
    /// `baumhard::util::color::hex_to_rgba_safe`.
    ///
    /// Comparing against the *effective* color rather than
    /// `style.text_color` is what makes this work on a themed node:
    /// there, `style.text_color` is a stale copy the palette
    /// shadows, and a run baked from the palette would never match
    /// it.
    ///
    /// On a **clear**, the baked runs are rewritten to the empty
    /// string rather than to the node's new effective color. That
    /// is the run tier's own spelling for "follow the node", so
    /// the graphemes rejoin the cascade instead of being re-pinned
    /// one tier down — which is the same trap `DEFAULT_RUN_COLOR`
    /// exists to avoid.
    ///
    /// **The un-bake alone is a change, so a clear can report
    /// `true` while nothing on screen moves.** On an unthemed node
    /// already sitting at
    /// [`DEFAULT_NODE_TEXT_COLOR`] the `style` half of the write
    /// is a genuine no-op, but a run carrying that same hex is
    /// opted *out* of the cascade and clearing it opts the
    /// graphemes back in: identical pixels today, and the next
    /// `color text=` or retheme carries them along instead of
    /// stranding them. Undo has to be able to put the bake back,
    /// which is why the entry is pushed rather than suppressed —
    /// "changed nothing on screen" is not the test, "changed
    /// nothing in the model" is, and the caller is told the truth
    /// about the model. A second clear finds nothing left to
    /// un-bake and is the no-op, so the gesture converges rather
    /// than growing the stack.
    pub fn set_node_text_color(&mut self, node_id: &str, color: Option<&str>) -> bool {
        self.set_node_color_channel(node_id, NodeColorChannel::Text, color)
    }

    /// Shared body of the three per-node color setters — the one
    /// place that decides *where* a per-node color write lands and
    /// what an absent color means once it gets there.
    ///
    /// Reads the effective pre-edit color through the same cascade
    /// the renderer uses, then writes the node's own
    /// `color_schema.overrides` when the node is themed and
    /// `node.style` when it is not. `NodeEditTail::None`: color
    /// never shifts a glyph advance, so there is nothing to
    /// re-measure.
    ///
    /// `color` is `None` for a clear, and an empty string counts
    /// as one on every channel where the empty string is not
    /// itself a color ([`NodeColorChannel::empty_is_a_color`]).
    /// Each tier spells a clear its own way: the override tier by
    /// dropping the slot, so the palette group shows through;
    /// `style`, which has nothing underneath it, by naming
    /// [`NodeColorChannel::unthemed_default`].
    ///
    /// Returns `true` when it wrote anything.
    fn set_node_color_channel(
        &mut self,
        node_id: &str,
        channel: NodeColorChannel,
        color: Option<&str>,
    ) -> bool {
        let Some(node) = self.mindmap.nodes.get(node_id) else {
            return false;
        };
        // Captured before the envelope's mutable borrow, and read
        // through the cascade rather than off `style`: on a themed
        // node `style.text_color` is a value the palette shadows,
        // so a run baked from the palette would never match it and
        // the whole node would refuse to recolor.
        let effective_before = match channel {
            NodeColorChannel::Background => self.mindmap.node_background_color(node),
            NodeColorChannel::Frame => self.mindmap.node_frame_color(node),
            NodeColorChannel::Text => self.mindmap.node_text_color(node),
        }
        .to_string();

        let write = match color {
            Some(c) if !c.is_empty() || channel.empty_is_a_color() => NodeColorWrite::Set(c),
            _ => NodeColorWrite::Clear,
        };
        // Both halves of the write are settled here, off the
        // mutable borrow, so the closure below only compares and
        // assigns. `run_target` is what a baked run follows the
        // node to: the new color when there is one, and the run
        // tier's "defer to the node" empty string on a clear.
        let themed_slot = match write {
            NodeColorWrite::Set(c) => Some(c.to_string()),
            NodeColorWrite::Clear => None,
        };
        let style_value = match write {
            NodeColorWrite::Set(c) => c.to_string(),
            NodeColorWrite::Clear => channel.unthemed_default().to_string(),
        };
        let run_target = match write {
            NodeColorWrite::Set(c) => c.to_string(),
            NodeColorWrite::Clear => String::new(),
        };

        self.mutate_node_with_style_undo(node_id, NodeEditTail::None, move |node| {
            let mut changed = match node.color_schema.as_mut() {
                // Themed: the palette group shadows `style`
                // entirely, so `style` is not where the read path
                // looks and not where the write may land.
                Some(schema) => {
                    let slot = match channel {
                        NodeColorChannel::Background => &mut schema.overrides.background,
                        NodeColorChannel::Frame => &mut schema.overrides.frame,
                        NodeColorChannel::Text => &mut schema.overrides.text,
                    };
                    let differs = *slot != themed_slot;
                    if differs {
                        *slot = themed_slot.clone();
                    }
                    differs
                }
                // Unthemed: `style` is the only tier there is.
                None => {
                    let slot = match channel {
                        NodeColorChannel::Background => &mut node.style.background_color,
                        NodeColorChannel::Frame => &mut node.style.frame_color,
                        NodeColorChannel::Text => &mut node.style.text_color,
                    };
                    let differs = *slot != style_value;
                    if differs {
                        slot.clone_from(&style_value);
                    }
                    differs
                }
            };
            // Runs that carry a baked copy of the old default
            // follow the node; runs that left `color` empty follow
            // it already and must keep doing so; everything else is
            // a deliberate per-span override.
            if channel == NodeColorChannel::Text {
                for section in node.sections.iter_mut() {
                    clamp_runs_to_text(section);
                    for run in section.text_runs.iter_mut() {
                        if !run.color.is_empty() && run.color == effective_before && run.color != run_target {
                            run.color.clone_from(&run_target);
                            changed = true;
                        }
                    }
                }
            }
            changed.then_some(())
        })
        .is_some()
    }

    /// Set the *default* font size on a node. Rewrites every
    /// `TextRun.size_pt` to `size_pt` — the node's runs all track
    /// the same size-in-points; unlike text color, there is no
    /// natural "keep per-run override" rule here (authored multi-
    /// size runs would already have been flattened by the text
    /// editor's collapse step in `set_node_text`).
    ///
    /// `size_pt` is clamped into the domain the loader accepts —
    /// floor 1, ceiling `MAX_FONT_SIZE_PT`. Both ends matter: below
    /// the floor a run shrinks into invisibility, and above the
    /// ceiling the loader refuses the saved file. Fractional sizes
    /// are stored as written — `size_pt` is an `f32` in the model.
    /// See `custom::sync::clamp_run_size_pt`.
    ///
    /// A section carrying text but no runs — a loadable shape,
    /// see `MindSection::text_runs` — gets one authored at the
    /// requested size rather than being skipped. It used to be
    /// skipped, silently and while returning `false`, because
    /// `all` over an empty run list is vacuously true; see
    /// `write_every_section_run`. A section with no text at all
    /// stays run-less: there is nothing to size, and a
    /// zero-length run is a shape `text_run_ops` panics on.
    pub fn set_node_font_size(&mut self, node_id: &str, size_pt: f32) -> bool {
        if !size_pt.is_finite() {
            return false;
        }
        // Clamped into the loader's run domain: the console's
        // `font size=` takes any positive finite f32, and a value
        // past the ceiling writes a map that will not reopen. See
        // `custom::sync::clamp_run_size_pt`.
        let size = crate::application::document::custom::sync::clamp_run_size_pt(size_pt);
        // `NodeEditTail::Grow`: larger text needs a larger box.
        // Monotonic floor — grow on demand, never shrink.
        self.mutate_node_with_style_undo(node_id, NodeEditTail::Grow, |node| {
            let already = node
                .sections
                .iter()
                .all(|s| section_runs_all_match(s, |r| r.size_pt == size));
            if already {
                return None;
            }
            for section in node.sections.iter_mut() {
                write_every_section_run(section, |r| r.size_pt = size);
            }
            Some(())
        })
        .is_some()
    }

    /// Set the font family on every `TextRun` of `node_id` to
    /// `family`. Returns `true` if any run actually changed.
    ///
    /// `Some(name)` pins each run to that family; `None` clears the
    /// pin by writing an empty string into each `TextRun.font` —
    /// which the tree builder treats as "fall back to the document
    /// default at render time" (`baumhard::mindmap::tree_builder::node`
    /// resolves empty-string font as `None` on the
    /// `ColorFontRegion`). Family-name validation is the caller's
    /// job; an unknown family lands in the data model and degrades
    /// at render time per CODE_CONVENTIONS §9.
    ///
    /// A run-less section carrying text gets a run authored at
    /// the requested family, on the same terms as
    /// [`Self::set_node_font_size`].
    ///
    /// Capture / undo: piggybacks on the existing
    /// `UndoAction::EditNodeStyle` envelope (which already includes
    /// the full `text_runs` snapshot via `before_runs`), so a
    /// `font set` on a node is reversed by the same `undo()` arm
    /// that reverses every other node-style edit. No new
    /// `UndoAction` variant.
    pub fn set_node_font_family(&mut self, node_id: &str, family: Option<&str>) -> bool {
        let target = family.unwrap_or("").to_string();
        // `NodeEditTail::Grow`: fonts vary wildly in advance
        // width — pinning a wide display face on a node sized
        // for a narrow monospace would clip the text against the
        // right edge.
        self.mutate_node_with_style_undo(node_id, NodeEditTail::Grow, move |node| {
            let already = node
                .sections
                .iter()
                .all(|s| section_runs_all_match(s, |r| r.font == target));
            if already {
                return None;
            }
            for section in node.sections.iter_mut() {
                write_every_section_run(section, |r| r.font = target.clone());
            }
            Some(())
        })
        .is_some()
    }

    /// Write the node's zoom-visibility window. Each of `min` /
    /// `max` is an [`OptionEdit<f32>`]: `Keep` leaves the side
    /// untouched, `Clear` sets it to `None` (unbounded), `Set(v)`
    /// sets it to `Some(v)`. Returns `true` if either side
    /// actually changed.
    ///
    /// Inversion (`min > max` after the edit) is rejected as a
    /// no-op with `false`; the console surface catches this first,
    /// so this is a defensive guard for programmatic callers.
    /// Non-finite values are likewise rejected — the invariant
    /// mirrors
    /// [`ZoomVisibility::try_new`](baumhard::gfx_structs::zoom_visibility::ZoomVisibility::try_new).
    pub fn set_node_zoom_visibility(
        &mut self,
        node_id: &str,
        min: OptionEdit<f32>,
        max: OptionEdit<f32>,
    ) -> bool {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let before_min = node.min_zoom_to_render;
        let before_max = node.max_zoom_to_render;
        let new_min = min.apply(before_min);
        let new_max = max.apply(before_max);
        if !validate_zoom_pair(new_min, new_max) {
            return false;
        }
        if new_min == before_min && new_max == before_max {
            return false;
        }
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        node.min_zoom_to_render = new_min;
        node.max_zoom_to_render = new_max;
        self.undo_stack.push(UndoAction::EditNodeZoom {
            node_id: node_id.to_string(),
            before_min,
            before_max,
        });
        self.dirty = true;
        true
    }
}

/// Validate a candidate `(x, y)` for a node: finite components,
/// each inside `MAX_CANVAS_COORD`. Nodes float freely on the canvas
/// (no parent AABB), so negative coordinates are legal — a node can
/// sit at a negative canvas-x to the left of the origin — but the
/// *magnitude* is bounded, because the loader bounds it too.
// Native-driver-only: guards the native drag/resize commits.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn validate_node_position(pos: baumhard::mindmap::model::Position) -> Result<(), String> {
    use baumhard::mindmap::model::validate::MAX_CANVAS_COORD;
    if !pos.x.is_finite() || !pos.y.is_finite() {
        return Err(format!(
            "node.position has non-finite component (x={}, y={})",
            pos.x, pos.y
        ));
    }
    // Magnitude too, not just finiteness. The loader bounds
    // `node.position` at `MAX_CANVAS_COORD`, so a setter that took
    // any finite `f64` let the editor write a map it would then
    // refuse to reopen — `set_node_aabb` with `x: 1e30` returned
    // `Ok(true)` and the saved file would not load.
    //
    // Rejected rather than clamped, unlike the font metrics: this
    // guard already returns `Result` and its callers already refuse
    // a bad position, so silently relocating a node to the boundary
    // would be a bigger surprise than declining the write. (It is
    // also why `validate::clamp_to_bound` is not used here — it
    // takes `f32` and these coordinates are `f64`.)
    for (name, value) in [("x", pos.x), ("y", pos.y)] {
        if value.abs() > MAX_CANVAS_COORD {
            return Err(format!(
                "node.position.{name} ({value}) is outside the ±{MAX_CANVAS_COORD} bound the \
                 loader enforces"
            ));
        }
    }
    Ok(())
}

/// Guard used by every `set_*_zoom_visibility` setter. Rejects a
/// pair whose bounds are non-finite or whose resolved
/// `(min, max)` inverts. Mirrors the contract the verifier
/// enforces at load time and `ZoomVisibility::try_new` enforces
/// for programmatic callers — no panic in interactive paths per
/// `CODE_CONVENTIONS.md` §9.
pub(super) fn validate_zoom_pair(min: Option<f32>, max: Option<f32>) -> bool {
    if let Some(m) = min {
        if !m.is_finite() {
            return false;
        }
    }
    if let Some(m) = max {
        if !m.is_finite() {
            return false;
        }
    }
    if let (Some(lo), Some(hi)) = (min, max) {
        if lo > hi {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::{
        first_testament_node_id as first_node_id, load_test_doc as fixture_doc,
    };

    /// `BorderConfigEdits::with_side_pattern` validates the
    /// pattern *before* mutating the bundle — a parse error
    /// leaves the slot untouched so a half-applied edit can't
    /// leak into the document. Critical for the verb's atomic
    /// contract.
    #[test]
    fn with_side_pattern_rejects_bad_input_without_mutation() {
        let mut edits = BorderConfigEdits::default();
        let err = edits
            .with_side_pattern(BorderSide::Top, "a)b")
            .expect_err("unmatched ')' must error");
        assert!(err.contains("top:"), "missing prefix: {}", err);
        assert!(matches!(edits.side_top, OptionEdit::Keep));
    }

    /// Setting a side pattern auto-promotes the preset to
    /// `"custom"` and surfaces that through `BorderEditOutcome`.
    /// The console verb consumes the `preset_auto_promoted` flag
    /// to print a note; this test guards the document-layer
    /// signal independently.
    #[test]
    fn set_node_border_config_signals_preset_auto_promotion() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        let mut edits = BorderConfigEdits {
            preset: OptionEdit::Set("heavy".into()),
            ..BorderConfigEdits::default()
        };
        edits
            .with_side_pattern(BorderSide::Top, "###(*)###")
            .expect("pattern parses");
        let outcome = doc.set_node_border_config(&id, edits);
        assert!(outcome.changed, "expected change applied");
        assert!(
            outcome.preset_auto_promoted,
            "side override against preset=heavy must auto-promote"
        );
        assert_eq!(outcome.requested_preset.as_deref(), Some("heavy"));
        let cfg = doc
            .mindmap
            .nodes
            .get(&id)
            .unwrap()
            .style
            .border
            .as_ref()
            .expect("config materialized");
        assert_eq!(cfg.preset, "custom");
    }

    /// `set_node_border_config` writes through the existing
    /// `EditNodeStyle` undo envelope so the next `undo()`
    /// restores the pre-edit `style.border`. Round-trip test:
    /// apply an edit, undo, confirm the override is gone (or
    /// matches its prior value).
    #[test]
    fn set_node_border_config_undo_round_trip_restores_style() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        let before_border = doc.mindmap.nodes.get(&id).unwrap().style.border.clone();
        let edits = BorderConfigEdits {
            preset: OptionEdit::Set("double".into()),
            ..BorderConfigEdits::default()
        };
        let outcome = doc.set_node_border_config(&id, edits);
        assert!(outcome.changed);
        // Sanity: the edit landed.
        assert_eq!(
            doc.mindmap
                .nodes
                .get(&id)
                .unwrap()
                .style
                .border
                .as_ref()
                .map(|c| c.preset.clone()),
            Some("double".to_string()),
        );
        // Now reverse.
        assert!(doc.undo(), "undo must succeed");
        let after_border = doc.mindmap.nodes.get(&id).unwrap().style.border.clone();
        assert_eq!(
            before_border.as_ref().map(|c| c.preset.clone()),
            after_border.as_ref().map(|c| c.preset.clone()),
            "undo must restore the pre-edit preset"
        );
    }

    /// `set_node_border_config` with `clear=true` on a node that
    /// already has no border override is a no-op — no undo
    /// entry, no `dirty` flag flip, returns `changed=false`.
    /// Guards the early-return branch.
    #[test]
    fn set_node_border_config_clear_no_op_when_already_none() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        // Strip any pre-existing override.
        doc.mindmap.nodes.get_mut(&id).unwrap().style.border = None;
        doc.dirty = false;
        let undo_len_before = doc.undo_stack.len();
        let edits = BorderConfigEdits {
            clear: true,
            ..BorderConfigEdits::default()
        };
        let outcome = doc.set_node_border_config(&id, edits);
        assert!(!outcome.changed);
        assert!(!doc.dirty, "no-op clear must not mark the document dirty");
        assert_eq!(
            doc.undo_stack.len(),
            undo_len_before,
            "no-op clear must not push an undo entry"
        );
    }

    /// `set_node_border_visible` toggles `style.show_frame` and
    /// returns `true` iff the value changed. Sibling test of
    /// the `set_*` patterns elsewhere in this module.
    #[test]
    fn set_node_border_visible_returns_true_only_on_change() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        // Force a known starting state.
        doc.mindmap.nodes.get_mut(&id).unwrap().style.show_frame = false;
        assert!(doc.set_node_border_visible(&id, true));
        assert!(doc.mindmap.nodes.get(&id).unwrap().style.show_frame);
        // Second call same value → no-op.
        assert!(!doc.set_node_border_visible(&id, true));
    }
}
