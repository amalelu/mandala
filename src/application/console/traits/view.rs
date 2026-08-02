// SPDX-License-Identifier: MPL-2.0

//! `TargetView` — enum holding a mutable doc reference plus
//! enough identity to find the targeted component. Capability-
//! trait impls and selection materialization live here.

use super::capabilities::{
    AcceptsFontFamily, AcceptsWheelColor, HandlesCopy, HandlesCut, HandlesPaste, HasBgColor, HasBorderColor,
    HasLabel, HasTextColor,
};
use super::color_value::ColorValue;
use super::outcome::{ClipboardContent, Outcome};
use crate::application::document::defaults::default_text_run;
use crate::application::document::{EdgeRef, MindMapDocument, SectionPayload, SelectionState};

/// A mutable view into one selected component, holding the doc ref
/// plus enough identity to find the component each time. Built fresh
/// per-iteration in a `Multi` fanout so no two views hold aliasing
/// `&mut doc` borrows at once.
pub enum TargetView<'a> {
    Node {
        doc: &'a mut MindMapDocument,
        id: String,
    },
    /// One section of one node — clipboard / per-section style
    /// dispatch routes here from `SelectionState::Section`. Copy
    /// reads `section.text` only (vs whole-node `display_text`
    /// for `Node`), paste / cut write through `set_section_text`
    /// to the indexed section. Color (text axis) and font
    /// (size + family) write per-section through the matching
    /// `set_section_*` setters; bg / border / zoom remain
    /// node-level (sections have no chrome by spec — see
    /// `format/sections.md`) and the Section arms in those traits
    /// return `Outcome::NotApplicable`.
    Section {
        doc: &'a mut MindMapDocument,
        id: String,
        section_idx: usize,
        /// Optional sub-range `[start, end)` over the section's
        /// grapheme indices. `Some(range)` routes per-section
        /// style dispatch (color text, font size, font family)
        /// to the range-aware setter; clipboard ops fall back
        /// to whole-section semantics (range-aware paste is
        /// deferred to N4-D). `None` = whole-section, the
        /// pre-N4-C semantic.
        range: Option<(usize, usize)>,
    },
    /// Line-mode edge body target. Color operations write the
    /// edge's `color` / `glyph_connection.color`; clipboard
    /// copy/paste/cut target the resolved **edge color** hex
    /// (the user's mental model of "copy this edge's colour").
    Edge {
        doc: &'a mut MindMapDocument,
        er: EdgeRef,
    },
    /// Line-mode **label** target. Carries the owning-edge ref
    /// so color writes route to `label_config.color` and
    /// clipboard operates on the resolved label color hex. The
    /// label text itself is edited through the inline modal,
    /// not through clipboard paste — paste of arbitrary text
    /// into the label would conflict with the color-hex
    /// paste semantics the user explicitly asked for.
    EdgeLabel {
        doc: &'a mut MindMapDocument,
        er: EdgeRef,
    },
    /// One endpoint's portal **icon** on a portal-mode edge.
    /// Carries both the owning-edge ref and the endpoint-node
    /// id so the trait arms can route mutations to the correct
    /// `PortalEndpointState.color` channel. Wheel / copy /
    /// paste / cut all operate on the icon's color.
    PortalLabel {
        doc: &'a mut MindMapDocument,
        er: EdgeRef,
        endpoint_node_id: String,
    },
    /// One endpoint's portal **text** on a portal-mode edge —
    /// the adjacent glyph area. Routes color writes to
    /// `PortalEndpointState.text_color` (independent from the
    /// icon) so a coloured badge can host a differently-coloured
    /// annotation. Clipboard operates on the resolved text
    /// color hex.
    PortalText {
        doc: &'a mut MindMapDocument,
        er: EdgeRef,
        endpoint_node_id: String,
    },
}

impl<'a> TargetView<'a> {
    /// One-word label, used in per-target error messages.
    pub fn kind(&self) -> &'static str {
        match self {
            TargetView::Node { .. } => "node",
            TargetView::Section { .. } => "section",
            TargetView::Edge { .. } => "edge",
            TargetView::EdgeLabel { .. } => "edge label",
            TargetView::PortalLabel { .. } => "portal label",
            TargetView::PortalText { .. } => "portal text",
        }
    }
}

/// Encode a ColorValue as the string the model field wants. `Reset`
/// resolves to `default` — each caller has its own "natural default"
/// string.
fn color_as_string(c: &ColorValue, default: &str) -> String {
    match c {
        ColorValue::Reset => default.to_string(),
        _ => c
            .as_model_string()
            .expect("non-reset ColorValue always encodes to a string"),
    }
}

/// Apply a color-override write to whichever edge-adjacent
/// variant `view` carries (`Edge` / `EdgeLabel` / `PortalLabel`
/// / `PortalText`). `override_str = None` clears the override
/// (lets the edge fall back to resolved config); `Some(s)` sets
/// it. Returns `NotApplicable` for non-edge variants — callers
/// route node / section variants separately.
///
/// Single source for the four-arm edge-adjacent dispatch shape
/// that `HasTextColor`, `HasBorderColor`, and `HandlesPaste`
/// (color-paste path) all need verbatim.
fn write_edge_adjacent_color(view: &mut TargetView, override_str: Option<&str>) -> Outcome {
    match view {
        TargetView::Edge { doc, er } => Outcome::applied(doc.set_edge_color(er, override_str)),
        TargetView::EdgeLabel { doc, er } => {
            Outcome::applied(doc.set_edge_label_color(er, override_str))
        }
        TargetView::PortalLabel {
            doc,
            er,
            endpoint_node_id,
        } => Outcome::applied(doc.set_portal_label_color(er, endpoint_node_id, override_str)),
        TargetView::PortalText {
            doc,
            er,
            endpoint_node_id,
        } => Outcome::applied(doc.set_portal_label_text_color(er, endpoint_node_id, override_str)),
        _ => Outcome::NotApplicable,
    }
}

/// Paste-of-color shape for the four edge-adjacent variants:
/// trim, accept empty as a clear, validate hex/var literal,
/// write through the same channel `set_text_color` /
/// `set_border_color` use. Single source for the
/// `HandlesPaste` edge arms.
fn paste_edge_adjacent_color(view: &mut TargetView, content: &str) -> Outcome {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return write_edge_adjacent_color(view, None);
    }
    if !is_valid_color_literal(trimmed) {
        return Outcome::Invalid(format!("not a color: {trimmed}"));
    }
    write_edge_adjacent_color(view, Some(trimmed))
}

/// Encode a ColorValue for the edge color path, where `None` means
/// "clear the override". Edges don't have a separate default string
/// — reset means fall back to resolved config.
fn edge_color_as_override(c: &ColorValue) -> Option<String> {
    match c {
        ColorValue::Reset => None,
        _ => Some(
            c.as_model_string()
                .expect("non-reset ColorValue always encodes to a string"),
        ),
    }
}

impl<'a> HasBgColor for TargetView<'a> {
    fn set_bg_color(&mut self, c: ColorValue) -> Outcome {
        match self {
            // Background fill is node-level chrome.
            TargetView::Node { doc, id } => {
                Outcome::applied(doc.set_node_bg_color(id, color_as_string(&c, "#141414")))
            }
            // Sections have no bg-fill chrome by spec
            // (`format/sections.md`). Matches `commands/color.rs`
            // where `color bg= section=K` already returns
            // NotApplicable.
            TargetView::Section { .. } => Outcome::NotApplicable,
            // Edges and all edge-sub-part selections have no
            // bg-fill concept — the body, label, icon, and text
            // each have one color, routed through `set_text_color`
            // / `set_border_color`. Reporting NotApplicable here
            // lets a multi-kv command like `color bg=#X text=#Y`
            // apply the text write to the selection without
            // failing on bg.
            TargetView::Edge { .. }
            | TargetView::EdgeLabel { .. }
            | TargetView::PortalLabel { .. }
            | TargetView::PortalText { .. } => Outcome::NotApplicable,
        }
    }
}

impl<'a> HasTextColor for TargetView<'a> {
    fn set_text_color(&mut self, c: ColorValue) -> Outcome {
        match self {
            // Whole-node `text` color rewrites the node's default
            // and every section's matching runs.
            TargetView::Node { doc, id } => {
                Outcome::applied(doc.set_node_text_color(id, color_as_string(&c, "#ffffff")))
            }
            TargetView::Section { doc, id, section_idx, range } => {
                let color_str = color_as_string(&c, "#ffffff");
                let applied = match range {
                    Some((rs, re)) => doc.set_section_text_color_range(id, *section_idx, *rs, *re, color_str),
                    None => doc.set_section_text_color(id, *section_idx, color_str),
                };
                Outcome::applied(applied)
            }
            // Edge / EdgeLabel / PortalLabel / PortalText share
            // the four-arm "one color channel per variant"
            // dispatch — single-sourced via
            // [`write_edge_adjacent_color`].
            TargetView::Edge { .. }
            | TargetView::EdgeLabel { .. }
            | TargetView::PortalLabel { .. }
            | TargetView::PortalText { .. } => {
                write_edge_adjacent_color(self, edge_color_as_override(&c).as_deref())
            }
        }
    }
}

impl<'a> HasBorderColor for TargetView<'a> {
    fn set_border_color(&mut self, c: ColorValue) -> Outcome {
        match self {
            // Frame/border is node-level chrome.
            TargetView::Node { doc, id } => {
                Outcome::applied(doc.set_node_border_color(id, color_as_string(&c, "#ffffff")))
            }
            // Sections have no frame/border chrome by spec
            // (`format/sections.md`).
            TargetView::Section { .. } => Outcome::NotApplicable,
            // `border` on any edge-adjacent selection is an alias
            // for `text` — each sub-part has one color channel
            // and the axis distinction doesn't apply. Routes
            // through the same single-sourced helper as
            // `HasTextColor` so `color border=` / `color text=`
            // stay interchangeable for these variants.
            TargetView::Edge { .. }
            | TargetView::EdgeLabel { .. }
            | TargetView::PortalLabel { .. }
            | TargetView::PortalText { .. } => {
                write_edge_adjacent_color(self, edge_color_as_override(&c).as_deref())
            }
        }
    }
}

impl<'a> AcceptsWheelColor for TargetView<'a> {
    fn apply_wheel_color(&mut self, c: ColorValue) -> Outcome {
        match self {
            // Node default: background fill.
            TargetView::Node { .. } => self.set_bg_color(c),
            // Section: text is the only colour axis a section has
            // (no bg/border chrome — see `HasBgColor`/`HasBorderColor`
            // arms above), so the undirected wheel commit routes
            // through `set_text_color` → `set_section_text_color`.
            TargetView::Section { .. } => self.set_text_color(c),
            // Every edge-adjacent selection routes the wheel
            // commit through `set_border_color`, which each
            // variant maps to its own one-channel color setter
            // (edge body / label / icon / text).
            TargetView::Edge { .. }
            | TargetView::EdgeLabel { .. }
            | TargetView::PortalLabel { .. }
            | TargetView::PortalText { .. } => self.set_border_color(c),
        }
    }
}

impl<'a> AcceptsFontFamily for TargetView<'a> {
    fn set_font_family(&mut self, family: Option<&str>) -> Outcome {
        match self {
            // Whole-node: writes every `TextRun.font` across every
            // section.
            TargetView::Node { doc, id } => Outcome::applied(doc.set_node_font_family(id, family)),
            // Section: per-section font family override, leaves
            // sibling sections' runs alone. With a `range` set,
            // routes to the range-aware setter instead.
            TargetView::Section { doc, id, section_idx, range } => {
                let applied = match range {
                    Some((rs, re)) => doc.set_section_font_family_range(id, *section_idx, *rs, *re, family),
                    None => doc.set_section_font_family(id, *section_idx, family),
                };
                Outcome::applied(applied)
            }
            // Edge body: `glyph_connection.font` override.
            TargetView::Edge { doc, er } => Outcome::applied(doc.set_edge_font_family(er, family)),
            // Portal icon shares the edge body's `glyph_connection.font` —
            // the same routing existing `font size=` uses for the
            // PortalLabel selection.
            TargetView::PortalLabel { doc, er, .. } => Outcome::applied(doc.set_edge_font_family(er, family)),
            // Edge labels and portal text inherit the edge body's
            // font today; no per-channel `font_family` slot exists
            // on `EdgeLabelConfig` / `PortalEndpointState` yet.
            TargetView::EdgeLabel { .. } | TargetView::PortalText { .. } => Outcome::NotApplicable,
        }
    }
}

impl<'a> HasLabel for TargetView<'a> {
    fn set_label(&mut self, s: Option<String>) -> Outcome {
        match self {
            // Edge and EdgeLabel both target the edge's `label`
            // field — selecting the sub-part explicitly doesn't
            // change what "set the label text" means; it's still
            // the same string on the same edge.
            TargetView::Edge { doc, er } | TargetView::EdgeLabel { doc, er } => {
                Outcome::applied(doc.set_edge_label(er, s))
            }
            // PortalLabel and PortalText both target the
            // endpoint's `text` field — same reasoning.
            TargetView::PortalLabel {
                doc,
                er,
                endpoint_node_id,
            }
            | TargetView::PortalText {
                doc,
                er,
                endpoint_node_id,
            } => Outcome::applied(doc.set_portal_label_text(er, endpoint_node_id, s)),
            _ => Outcome::NotApplicable,
        }
    }
}

impl<'a> HandlesCopy for TargetView<'a> {
    fn clipboard_copy(&self) -> ClipboardContent {
        match self {
            // Section: structured payload (`text` to OS clipboard,
            // `payload` to in-process buffer). Empty text still
            // emits `Section` because chrome may carry information.
            // Range-aware copy: when `range` is set, we fall
            // back to whole-section copy. The semantic is
            // safe for copy (non-destructive) but Cut+Paste below
            // explicitly reject the range to prevent surprise.
            TargetView::Section { doc, id, section_idx, .. } => match doc
                .mindmap
                .nodes
                .get(id)
                .and_then(|n| n.sections.get(*section_idx))
            {
                Some(section) => ClipboardContent::Section {
                    text: section.text.clone(),
                    payload: SectionPayload::from_section(section),
                },
                None => ClipboardContent::NotApplicable,
            },
            // Node copy = the node's current text (every section
            // joined by '\n' via `display_text`). Empty text
            // reports `Empty` so the caller can distinguish from a
            // target type that doesn't support copy at all.
            TargetView::Node { doc, id } => match doc.mindmap.nodes.get(id) {
                Some(n) => {
                    let text = n.display_text();
                    if text.is_empty() {
                        ClipboardContent::Empty
                    } else {
                        ClipboardContent::Text(text.into_owned())
                    }
                }
                None => ClipboardContent::NotApplicable,
            },
            // Edge copy = the resolved edge color hex. User-facing
            // spec: clipboard copy on an edge copies its colour
            // (changed from the prior label-text behaviour — edge
            // label text is edited through the inline modal, which
            // handles its own OS-clipboard surface).
            TargetView::Edge { doc, er } => match doc.resolve_edge_color(er) {
                Some(hex) => ClipboardContent::Text(hex),
                None => ClipboardContent::NotApplicable,
            },
            // Edge label copy = resolved label color hex (cascade:
            // label_config.color → glyph_connection.color →
            // edge.color). Always a concrete hex when the edge
            // exists, so pasting to another target produces a
            // real value.
            TargetView::EdgeLabel { doc, er } => match doc.resolve_edge_label_color(er) {
                Some(hex) => ClipboardContent::Text(hex),
                None => ClipboardContent::NotApplicable,
            },
            // Portal icon copy = resolved icon color hex. Always a
            // real value (cascade fallback resolves to a concrete
            // hex even when no override is set).
            TargetView::PortalLabel {
                doc,
                er,
                endpoint_node_id,
            } => match doc.resolve_portal_label_color(er, endpoint_node_id) {
                Some(hex) => ClipboardContent::Text(hex),
                None => ClipboardContent::NotApplicable,
            },
            // Portal text copy = resolved text color hex (cascade:
            // text_color → icon color cascade).
            TargetView::PortalText {
                doc,
                er,
                endpoint_node_id,
            } => match doc.resolve_portal_text_color(er, endpoint_node_id) {
                Some(hex) => ClipboardContent::Text(hex),
                None => ClipboardContent::NotApplicable,
            },
        }
    }
}

impl<'a> HandlesPaste for TargetView<'a> {
    fn clipboard_paste(&mut self, content: &str) -> Outcome {
        match self {
            // Section paste. Range-aware: when the selection is
            // SectionRange, replace only the in-range graphemes
            // with `content` (shifting later runs accordingly).
            // Whole-section: structured payload via the in-process
            // buffer when its snapshot matches the untrimmed
            // probe; falls back to plain-text template inheritance.
            TargetView::Section { doc, id, section_idx, range } => {
                if let Some((rs, re)) = *range {
                    return paste_section_range(doc, id, *section_idx, rs, re, content);
                }
                let section_count = doc
                    .mindmap
                    .nodes
                    .get(id.as_str())
                    .map(|n| n.sections.len())
                    .unwrap_or(0);
                if section_count == 0 {
                    return Outcome::NotApplicable;
                }
                let target = (*section_idx).min(section_count - 1);
                if let Some(payload) = crate::application::clipboard::read_section_clipboard(content) {
                    Outcome::applied(doc.apply_section_payload(id, target, content.to_string(), &payload))
                } else {
                    Outcome::applied(doc.set_section_text(id, target, content.trim_end().to_string()))
                }
            }
            // Paste replaces the node's text with the clipboard
            // contents wholesale. Today's `set_node_text` writes
            // section[0]; sections 1+ stay intact. Authors who
            // want full-node replacement should explicitly select
            // a section first.
            TargetView::Node { doc, id } => {
                Outcome::applied(doc.set_node_text(id, content.trim_end().to_string()))
            }
            // Edge / EdgeLabel / PortalLabel / PortalText paste:
            // trim → empty-as-clear → hex/var validation → write
            // through the variant's one color channel. Single-
            // sourced via [`paste_edge_adjacent_color`].
            TargetView::Edge { .. }
            | TargetView::EdgeLabel { .. }
            | TargetView::PortalLabel { .. }
            | TargetView::PortalText { .. } => paste_edge_adjacent_color(self, &content),
        }
    }
}

impl<'a> HandlesCut for TargetView<'a> {
    fn clipboard_cut(&mut self) -> ClipboardContent {
        match self {
            // Section cut: snapshot the structured payload, then
            // clear text + runs while preserving offset / size /
            // channel / bindings on the source section so the cut
            // reads as "the text disappeared" rather than "the
            // section dissolved." Range-aware cut removes only the
            // in-range graphemes, returns them as plain text, and
            // shifts later runs left.
            TargetView::Section { doc, id, section_idx, range } => {
                if let Some((rs, re)) = *range {
                    return cut_section_range(doc, id, *section_idx, rs, re);
                }
                let (text, payload) = match doc
                    .mindmap
                    .nodes
                    .get(id)
                    .and_then(|n| n.sections.get(*section_idx))
                {
                    Some(section) => (section.text.clone(), SectionPayload::from_section(section)),
                    None => return ClipboardContent::NotApplicable,
                };
                let cleared = SectionPayload {
                    text_runs: Vec::new(),
                    offset: payload.offset.clone(),
                    size: payload.size.clone(),
                    channel: payload.channel,
                    trigger_bindings: payload.trigger_bindings.clone(),
                };
                doc.apply_section_payload(id, *section_idx, String::new(), &cleared);
                ClipboardContent::Section { text, payload }
            }
            TargetView::Node { doc, id } => {
                let text = match doc.mindmap.nodes.get(id) {
                    Some(n) => n.display_text().into_owned(),
                    None => return ClipboardContent::NotApplicable,
                };
                // Clear **every** section's text — `clipboard_copy`
                // on this same target reads `display_text()` (every
                // section joined by `\n`), so cut must zero the
                // same scope. Pre-fix only `section[0]` was cleared
                // (via `set_node_text`), leaving zombie content in
                // `sections[1..]` that wasn't on the clipboard —
                // copy → cut → paste produced a corrupted node
                // with the joined text in `section[0]` and the
                // pre-cut `sections[1..]` text still in place.
                // Section count is preserved so subsequent
                // section-aware paste still has the same anchor
                // shape; structural round-trip across the joined
                // string is lossy on section boundaries (the
                // documented limit of `display_text()`).
                let section_count = doc.mindmap.nodes.get(id).map(|n| n.sections.len()).unwrap_or(0);
                for idx in 0..section_count {
                    doc.set_section_text(id, idx, String::new());
                }
                if text.is_empty() {
                    ClipboardContent::Empty
                } else {
                    ClipboardContent::Text(text)
                }
            }
            // Edge cut = resolved color hex + clear
            // `glyph_connection.color` override so the edge
            // reverts to its base `edge.color`. The user still
            // gets a real hex (cascade fallback always resolves
            // to one), but the visible edge body resets.
            TargetView::Edge { doc, er } => {
                let hex = doc.resolve_edge_color(er);
                doc.set_edge_color(er, None);
                match hex {
                    Some(h) => ClipboardContent::Text(h),
                    None => ClipboardContent::NotApplicable,
                }
            }
            // Edge label cut = resolved label color + clear
            // `label_config.color` override.
            TargetView::EdgeLabel { doc, er } => {
                let resolved = doc.resolve_edge_label_color(er);
                doc.set_edge_label_color(er, None);
                match resolved {
                    Some(hex) => ClipboardContent::Text(hex),
                    None => ClipboardContent::NotApplicable,
                }
            }
            // Portal icon cut = resolved icon color + clear
            // per-endpoint override. Label visually resets to
            // the edge color.
            TargetView::PortalLabel {
                doc,
                er,
                endpoint_node_id,
            } => {
                let resolved = doc.resolve_portal_label_color(er, endpoint_node_id);
                doc.set_portal_label_color(er, endpoint_node_id, None);
                match resolved {
                    Some(hex) => ClipboardContent::Text(hex),
                    None => ClipboardContent::NotApplicable,
                }
            }
            // Portal text cut = resolved text color + clear
            // per-endpoint `text_color` override. Text visually
            // resets to the icon color cascade.
            TargetView::PortalText {
                doc,
                er,
                endpoint_node_id,
            } => {
                let resolved = doc.resolve_portal_text_color(er, endpoint_node_id);
                doc.set_portal_label_text_color(er, endpoint_node_id, None);
                match resolved {
                    Some(hex) => ClipboardContent::Text(hex),
                    None => ClipboardContent::NotApplicable,
                }
            }
        }
    }
}

/// Minimal recognizer for the two color-literal forms the document
/// paste path accepts: `#`-prefixed Baumhard hex colors and
/// `var(--name)` theme references. Clipboard content is arbitrary
/// text, so paste requires the `#` prefix for hex even though
/// Baumhard's canonical parser also accepts bare hex.
fn is_valid_color_literal(s: &str) -> bool {
    (s.starts_with('#') && baumhard::util::color::is_valid_hex_color(s))
        || baumhard::util::color::is_var_ref(s)
}

fn read_edge_label(doc: &MindMapDocument, er: &EdgeRef) -> Option<String> {
    let idx = doc.edge_index(er)?;
    doc.mindmap.edges.get(idx).and_then(|e| e.label.clone())
}

/// Snapshot the selection into a list of target identities the
/// dispatcher can iterate over. Returns owned strings / refs so the
/// caller can build a fresh `TargetView` per iteration (aliasing-
/// safe fanout).
pub enum TargetId {
    Node(String),
    /// One section of one node, identified by `(node_id, section_idx)`.
    /// Surfaces only for `SelectionState::Section`; clipboard copy /
    /// cut / paste route to the specific section's `text` (vs the
    /// whole-node `display_text` join for `Node` targets).
    Section {
        node_id: String,
        section_idx: usize,
        /// Optional sub-range `[start, end)` over the section's
        /// grapheme indices — emitted when `selection_targets`
        /// fans `SelectionState::SectionRange` out. The trait
        /// dispatcher's `TargetView::Section { range, .. }`
        /// arm consults it.
        range: Option<(usize, usize)>,
    },
    Edge(EdgeRef),
    EdgeLabel(EdgeRef),
    PortalLabel {
        edge: EdgeRef,
        endpoint_node_id: String,
    },
    PortalText {
        edge: EdgeRef,
        endpoint_node_id: String,
    },
}

pub fn selection_targets(sel: &SelectionState) -> Vec<TargetId> {
    match sel {
        SelectionState::None => Vec::new(),
        SelectionState::Single(id) => vec![TargetId::Node(id.clone())],
        SelectionState::Multi(ids) => ids.iter().cloned().map(TargetId::Node).collect(),
        // A section selection routes to a dedicated `Section`
        // target. Per-trait behaviour at the dispatch site:
        // clipboard copy / cut / paste land on the section's
        // `text`; color (text axis) and font (size + family) write
        // per-section through the matching `set_section_*`
        // setters; bg / border / zoom return `NotApplicable` for
        // the Section arm because sections have no chrome by spec
        // (see `format/sections.md`).
        SelectionState::Section(s) => vec![TargetId::Section {
            node_id: s.node_id.clone(),
            section_idx: s.section_idx,
            range: None,
        }],
        // Multi-section fans out to one `Section` target per
        // entry — every per-section verb (colour text axis,
        // font size / family, clipboard text) applies to each.
        SelectionState::MultiSection(secs) => secs
            .iter()
            .map(|s| TargetId::Section {
                node_id: s.node_id.clone(),
                section_idx: s.section_idx,
                range: None,
            })
            .collect(),
        // SectionRange routes through the same `Section`
        // target so per-section verbs reuse one dispatch arm;
        // the carried range threads to range-aware setters
        // inside the dispatcher.
        SelectionState::SectionRange { sel, range } => vec![TargetId::Section {
            node_id: sel.node_id.clone(),
            section_idx: sel.section_idx,
            range: Some(*range),
        }],
        SelectionState::Edge(er) => vec![TargetId::Edge(er.clone())],
        SelectionState::EdgeLabel(s) => vec![TargetId::EdgeLabel(s.edge_ref.clone())],
        SelectionState::PortalLabel(s) => vec![TargetId::PortalLabel {
            edge: s.edge_ref(),
            endpoint_node_id: s.endpoint_node_id.clone(),
        }],
        SelectionState::PortalText(s) => vec![TargetId::PortalText {
            edge: s.edge_ref(),
            endpoint_node_id: s.endpoint_node_id.clone(),
        }],
    }
}

/// Rebuild a `TargetView` on a fresh `&mut doc` borrow. Call this
/// once per iteration of the fanout loop so no two views overlap.
pub fn view_for<'a>(doc: &'a mut MindMapDocument, id: &TargetId) -> TargetView<'a> {
    match id {
        TargetId::Node(nid) => TargetView::Node { doc, id: nid.clone() },
        TargetId::Section { node_id, section_idx, range } => TargetView::Section {
            doc,
            id: node_id.clone(),
            section_idx: *section_idx,
            range: *range,
        },
        TargetId::Edge(er) => TargetView::Edge { doc, er: er.clone() },
        TargetId::EdgeLabel(er) => TargetView::EdgeLabel { doc, er: er.clone() },
        TargetId::PortalLabel {
            edge,
            endpoint_node_id,
        } => TargetView::PortalLabel {
            doc,
            er: edge.clone(),
            endpoint_node_id: endpoint_node_id.clone(),
        },
        TargetId::PortalText {
            edge,
            endpoint_node_id,
        } => TargetView::PortalText {
            doc,
            er: edge.clone(),
            endpoint_node_id: endpoint_node_id.clone(),
        },
    }
}

/// Range-aware Cut: drop the in-range graphemes from one
/// section's text + runs, return them as plain text. Pure
/// `ClipboardContent::Text` rather than `Section { text, payload }`
/// because the structured payload's geometry / channel /
/// trigger_bindings belong to the *whole* section, not a
/// sub-range. No-op when the section is missing or the range is
/// empty after clamping.
fn cut_section_range(
    doc: &mut MindMapDocument,
    node_id: &str,
    section_idx: usize,
    range_start: usize,
    range_end: usize,
) -> ClipboardContent {
    use baumhard::util::grapheme_chad;
    let Some(node) = doc.mindmap.nodes.get(node_id) else {
        return ClipboardContent::NotApplicable;
    };
    let Some(section) = node.sections.get(section_idx) else {
        return ClipboardContent::NotApplicable;
    };
    let total = grapheme_chad::count_grapheme_clusters(&section.text);
    let clamped_end = range_end.min(total);
    if range_start >= clamped_end {
        return ClipboardContent::Empty;
    }
    let byte_start = grapheme_chad::find_byte_index_of_grapheme(&section.text, range_start)
        .unwrap_or(section.text.len());
    let byte_end = grapheme_chad::find_byte_index_of_grapheme(&section.text, clamped_end)
        .unwrap_or(section.text.len());
    let cut_text = section.text[byte_start..byte_end].to_string();
    let mut new_text = String::with_capacity(section.text.len() - (byte_end - byte_start));
    new_text.push_str(&section.text[..byte_start]);
    new_text.push_str(&section.text[byte_end..]);
    let mut new_runs = section.text_runs.clone();
    let template = section
        .text_runs
        .first()
        .cloned()
        .unwrap_or_else(|| baumhard::mindmap::model::TextRun {
            color: node.style.text_color.clone(),
            ..default_text_run(0)
        });
    baumhard::mindmap::model::text_run_ops::splice_range(
        &mut new_runs,
        range_start,
        clamped_end,
        0,
        &template,
    );
    let payload = SectionPayload {
        text_runs: new_runs,
        offset: section.offset.clone(),
        size: section.size.clone(),
        channel: section.channel,
        trigger_bindings: section.trigger_bindings.clone(),
    };
    doc.apply_section_payload(node_id, section_idx, new_text, &payload);
    ClipboardContent::Text(cut_text)
}

/// Range-aware Paste: replace the in-range graphemes of one
/// section's text with `content`, threading style from the
/// section's first run (cascade source). Returns `Outcome::Applied`
/// on a real change, `Outcome::NotApplicable` when the section
/// is missing.
fn paste_section_range(
    doc: &mut MindMapDocument,
    node_id: &str,
    section_idx: usize,
    range_start: usize,
    range_end: usize,
    content: &str,
) -> Outcome {
    use baumhard::util::grapheme_chad;
    let Some(node) = doc.mindmap.nodes.get(node_id) else {
        return Outcome::NotApplicable;
    };
    let Some(section) = node.sections.get(section_idx) else {
        return Outcome::NotApplicable;
    };
    let total = grapheme_chad::count_grapheme_clusters(&section.text);
    let clamped_end = range_end.min(total);
    if range_start > clamped_end {
        return Outcome::NotApplicable;
    }
    let byte_start = grapheme_chad::find_byte_index_of_grapheme(&section.text, range_start)
        .unwrap_or(section.text.len());
    let byte_end = grapheme_chad::find_byte_index_of_grapheme(&section.text, clamped_end)
        .unwrap_or(section.text.len());
    let mut new_text = String::with_capacity(
        section.text.len() - (byte_end - byte_start) + content.len(),
    );
    new_text.push_str(&section.text[..byte_start]);
    new_text.push_str(content);
    new_text.push_str(&section.text[byte_end..]);
    let content_grapheme_count = grapheme_chad::count_grapheme_clusters(content);
    let mut new_runs = section.text_runs.clone();
    // Inherit style from the run at the insertion point — for a
    // paste in the middle of `[(0,3) red, (3,8) blue]` at idx 5
    // the new content takes blue, not red. Falls back to the
    // section's first run for boundary cases (insertion at a
    // gap), and to a hardcoded default when the section has no
    // runs.
    let template = baumhard::mindmap::model::text_run_ops::find_run_containing(
        &section.text_runs,
        range_start,
    )
    .or_else(|| {
        // At a run boundary `find_run_containing` returns None;
        // prefer the run ending at the boundary (left neighbour)
        // since the user is conceptually typing "after" it.
        if range_start == 0 {
            None
        } else {
            baumhard::mindmap::model::text_run_ops::find_run_containing(
                &section.text_runs,
                range_start - 1,
            )
        }
    })
    .map(|idx| section.text_runs[idx].clone())
    .or_else(|| section.text_runs.first().cloned())
    .unwrap_or_else(|| baumhard::mindmap::model::TextRun {
        color: node.style.text_color.clone(),
        ..default_text_run(0)
    });
    baumhard::mindmap::model::text_run_ops::splice_range(
        &mut new_runs,
        range_start,
        clamped_end,
        content_grapheme_count,
        &template,
    );
    let payload = SectionPayload {
        text_runs: new_runs,
        offset: section.offset.clone(),
        size: section.size.clone(),
        channel: section.channel,
        trigger_bindings: section.trigger_bindings.clone(),
    };
    Outcome::applied(doc.apply_section_payload(node_id, section_idx, new_text, &payload))
}
