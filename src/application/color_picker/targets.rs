// SPDX-License-Identifier: MPL-2.0

//! What the picker is currently editing, and how to resolve / read
//! the live color off the document for that target. Two target
//! classes today — `Edge` (covers both line-mode and portal-mode
//! edges, since portals are a `display_mode` on the same entity)
//! and `Node{axis}` — each carried as a palette-to-picker handoff
//! value (`ColorTarget`) and then as a resolved, stable handle
//! (`PickerHandle`) once the picker is open.

use baumhard::util::color::{hex_to_hsv_safe, resolve_var};

use crate::application::document::{EdgeRef, MindMapDocument};

/// Which visual axis on a node the picker should write to when the
/// target is a node. Edges don't need this — they have one color
/// field (plus an optional `glyph_connection.color` override).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeColorAxis {
    Bg,
    Text,
    Border,
}

/// Which visual axis on a section the picker should write to. Today
/// sections only have a text colour axis (no bg/border chrome by
/// spec — see `format/sections.md` and the `HasBgColor` /
/// `HasBorderColor` trait arms in `console/traits/view.rs`).
/// Single-variant on purpose so adding `Bg` / `Border` later (only
/// if the data shape changes) is a non-breaking extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionColorAxis {
    Text,
}

/// Palette-to-picker handoff value. Carries an unresolved reference
/// to the thing the picker is about to edit — the picker resolves
/// this once at open time into a `PickerHandle` and then forgets the
/// original ref so the hot hover path never has to re-search.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorTarget {
    Edge(EdgeRef),
    Node {
        id: String,
        axis: NodeColorAxis,
    },
    /// One section of one node — picker writes through
    /// `set_section_text_color` so sibling sections stay
    /// untouched. Only emitted when the active selection is
    /// `SelectionState::Section` and the verb's axis maps to
    /// `SectionColorAxis::Text`.
    Section {
        node_id: String,
        section_idx: usize,
        axis: SectionColorAxis,
        /// Optional sub-range over the section's grapheme
        /// indices. Set when the active selection is
        /// `SelectionState::SectionRange`; unset for whole-section
        /// `Section`. The picker's commit path routes through
        /// `set_section_text_color_range` when present.
        range: Option<(usize, usize)>,
    },
}

/// Resolved handle carried inside `ColorPickerState::Open`. For
/// edges it indexes into the live `Vec`; for nodes it carries the
/// id + axis directly. One enum instead of `kind + target_index` +
/// a parallel optional id field.
#[derive(Clone, Debug)]
pub enum PickerHandle {
    Edge(usize),
    Node {
        id: String,
        axis: NodeColorAxis,
    },
    /// Section handle — node id + section index + axis. The
    /// resolve step verifies the node and the section index still
    /// exist; the index is captured at open time and held until
    /// commit (mirrors the Edge variant's stale-index defensive
    /// pattern). `range` carries the sub-range from a
    /// `SelectionState::SectionRange` selection at open time;
    /// the commit routes through `set_section_text_color_range`
    /// when present.
    Section {
        node_id: String,
        section_idx: usize,
        axis: SectionColorAxis,
        range: Option<(usize, usize)>,
    },
}

impl PickerHandle {
    /// Short label for the picker title bar. Portal-mode edges and
    /// line-mode edges both read as "edge" here — the display mode
    /// is already visible in the canvas, so repeating it in the
    /// picker chrome would be noise.
    pub fn label(&self) -> &'static str {
        match self {
            PickerHandle::Edge(_) => "edge",
            PickerHandle::Node { .. } => "node",
            PickerHandle::Section { .. } => "section",
        }
    }

    pub fn kind(&self) -> TargetKind {
        match self {
            PickerHandle::Edge(_) => TargetKind::Edge,
            PickerHandle::Node { .. } => TargetKind::Node,
            PickerHandle::Section { .. } => TargetKind::Section,
        }
    }
}

/// Coarse target kind for legacy call-sites that only need to
/// distinguish edges / nodes / sections without caring about the
/// concrete id or axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Edge,
    Node,
    Section,
}

impl TargetKind {
    /// Short label for the picker title bar.
    pub fn label(&self) -> &'static str {
        match self {
            TargetKind::Edge => "edge",
            TargetKind::Node => "node",
            TargetKind::Section => "section",
        }
    }
}

impl ColorTarget {
    /// Resolve the target ref to a concrete [`PickerHandle`]. Returns
    /// `None` if the underlying edge / node was deleted between the
    /// open trigger and the picker-open call (should never happen in
    /// practice because the modal holds the event loop, but
    /// defensive).
    pub fn resolve(self, doc: &MindMapDocument) -> Option<PickerHandle> {
        match self {
            ColorTarget::Edge(er) => doc
                .mindmap
                .edges
                .iter()
                .position(|e| er.matches(e))
                .map(PickerHandle::Edge),
            ColorTarget::Node { id, axis } => doc
                .mindmap
                .nodes
                .contains_key(&id)
                .then_some(PickerHandle::Node { id, axis }),
            ColorTarget::Section {
                node_id,
                section_idx,
                axis,
                range,
            } => {
                // Verify the section index still resolves — a
                // mutation between the open trigger and resolve
                // could have shrunk `node.sections` below the
                // captured index. Mirrors the Edge variant's
                // stale-ref defensive check.
                let exists = doc
                    .mindmap
                    .nodes
                    .get(&node_id)
                    .map(|n| section_idx < n.sections.len())
                    .unwrap_or(false);
                exists.then_some(PickerHandle::Section {
                    node_id,
                    section_idx,
                    axis,
                    range,
                })
            }
        }
    }
}

/// Read the current color string for a handle. Used to seed picker
/// HSV at open time and to read the effective color for the
/// preview after a chip action. Returns `None` if the index / id
/// no longer resolves.
pub fn current_color_at(doc: &MindMapDocument, handle: &PickerHandle) -> Option<String> {
    match handle {
        PickerHandle::Edge(index) => {
            let e = doc.mindmap.edges.get(*index)?;
            Some(
                e.glyph_connection
                    .as_ref()
                    .and_then(|gc| gc.color.clone())
                    .unwrap_or_else(|| e.color.clone()),
            )
        }
        PickerHandle::Node { id, axis } => {
            let n = doc.mindmap.nodes.get(id)?;
            Some(match axis {
                NodeColorAxis::Bg => n.style.background_color.clone(),
                NodeColorAxis::Text => n.style.text_color.clone(),
                NodeColorAxis::Border => n.style.frame_color.clone(),
            })
        }
        PickerHandle::Section {
            node_id,
            section_idx,
            axis,
            range,
        } => {
            let n = doc.mindmap.nodes.get(node_id)?;
            let section = n.sections.get(*section_idx)?;
            let resolved = match axis {
                SectionColorAxis::Text => match range {
                    Some((rs, re)) => {
                        let in_range =
                            baumhard::mindmap::model::text_run_ops::slice(&section.text_runs, *rs, *re);
                        // Coverage check: the in-range slice must
                        // span the entire `[rs, re)` with no gaps
                        // for "unanimous" to be meaningful — a
                        // partially-covered range mixes the
                        // covered run's colour with the gap's
                        // node-default fall-through, so the
                        // user's effective colour in the range
                        // is *not* unanimous. Without this
                        // check, a single in-range run would
                        // pass the trivial `iter().all` and
                        // seed the picker with the wrong colour.
                        let fully_covered = in_range.first().is_some_and(|first| first.start == *rs)
                            && in_range.last().is_some_and(|last| last.end == *re)
                            && in_range.windows(2).all(|w| w[0].end == w[1].start);
                        let unanimous = fully_covered
                            && in_range
                                .first()
                                .is_some_and(|first| in_range.iter().all(|r| r.color == first.color));
                        if unanimous {
                            in_range
                                .first()
                                .map(|r| r.color.clone())
                                .unwrap_or_else(|| n.style.text_color.clone())
                        } else {
                            n.style.text_color.clone()
                        }
                    }
                    None => section
                        .text_runs
                        .first()
                        .filter(|first| section.text_runs.iter().all(|r| r.color == first.color))
                        .map(|r| r.color.clone())
                        .unwrap_or_else(|| n.style.text_color.clone()),
                },
            };
            Some(resolved)
        }
    }
}

/// Resolve the current color through the canvas theme variables and
/// parse it into HSV for seeding the picker state. Falls back to
/// `(0.0, 0.0, 0.5)` (mid-gray) on any failure so the picker always
/// opens with a sensible default.
pub fn current_hsv_at(doc: &MindMapDocument, handle: &PickerHandle) -> (f32, f32, f32) {
    let raw = match current_color_at(doc, handle) {
        Some(s) => s,
        None => return (0.0, 0.0, 0.5),
    };
    let resolved = resolve_var(&raw, &doc.mindmap.canvas.theme_variables);
    hex_to_hsv_safe(resolved).unwrap_or((0.0, 0.0, 0.5))
}
