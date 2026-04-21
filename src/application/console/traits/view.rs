//! `TargetView` — the enum that holds a mutable doc reference plus
//! enough identity to find the component each iteration. All
//! capability-trait impls live here; selection materialization
//! (`selection_targets`, `view_for`) sits with the view since each
//! is a single-line constructor.
//!
//! Three target shapes: `Node`, `Edge`, and `PortalLabel`.
//! Portal-mode edges go through the `Edge` shape just like
//! line-mode edges — `display_mode` is a render flag, not a
//! separate entity, so trait dispatch doesn't split on it.
//! `PortalLabel` is its own variant because its trait impls
//! route to the per-endpoint `PortalEndpointState` instead of
//! the owning edge's fields.

use super::capabilities::{
    AcceptsWheelColor, HandlesCopy, HandlesCut, HandlesPaste, HasBgColor, HasBorderColor,
    HasFontSize, HasLabel, HasTextColor,
};
use super::color_value::ColorValue;
use super::outcome::{ClipboardContent, Outcome};
use crate::application::document::{EdgeRef, MindMapDocument, SelectionState};

/// A mutable view into one selected component, holding the doc ref
/// plus enough identity to find the component each time. Built fresh
/// per-iteration in a `Multi` fanout so no two views hold aliasing
/// `&mut doc` borrows at once.
pub enum TargetView<'a> {
    Node { doc: &'a mut MindMapDocument, id: String },
    /// Line-mode edge body target. Color operations write the
    /// edge's `color` / `glyph_connection.color`; clipboard
    /// copy/paste/cut target the resolved **edge color** hex
    /// (the user's mental model of "copy this edge's colour").
    Edge { doc: &'a mut MindMapDocument, er: EdgeRef },
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
            TargetView::Node { doc, id } => {
                Outcome::applied(doc.set_node_bg_color(id, color_as_string(&c, "#141414")))
            }
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
            TargetView::Node { doc, id } => {
                Outcome::applied(doc.set_node_text_color(id, color_as_string(&c, "#ffffff")))
            }
            // Edge body: the edge's one color field (line + any
            // text that inherits).
            TargetView::Edge { doc, er } => Outcome::applied(
                doc.set_edge_color(er, edge_color_as_override(&c).as_deref()),
            ),
            // Edge label: the label's own color override — lets a
            // coloured edge carry a differently-coloured label
            // (the user-facing independent-label-color feature).
            TargetView::EdgeLabel { doc, er } => Outcome::applied(
                doc.set_edge_label_color(er, edge_color_as_override(&c).as_deref()),
            ),
            TargetView::PortalLabel {
                doc,
                er,
                endpoint_node_id,
            } => Outcome::applied(doc.set_portal_label_color(
                er,
                endpoint_node_id,
                edge_color_as_override(&c).as_deref(),
            )),
            // Portal text: the per-endpoint text color override,
            // independent from the icon.
            TargetView::PortalText {
                doc,
                er,
                endpoint_node_id,
            } => Outcome::applied(doc.set_portal_label_text_color(
                er,
                endpoint_node_id,
                edge_color_as_override(&c).as_deref(),
            )),
        }
    }
}

impl<'a> HasBorderColor for TargetView<'a> {
    fn set_border_color(&mut self, c: ColorValue) -> Outcome {
        match self {
            TargetView::Node { doc, id } => Outcome::applied(
                doc.set_node_border_color(id, color_as_string(&c, "#ffffff")),
            ),
            // `border` on any edge-adjacent selection is an alias
            // for `text` — each sub-part has one color channel
            // and the axis distinction doesn't apply. Routing
            // through the same setters keeps the console's `color
            // border=` / `color text=` pair interchangeable for
            // these variants.
            TargetView::Edge { doc, er } => Outcome::applied(
                doc.set_edge_color(er, edge_color_as_override(&c).as_deref()),
            ),
            TargetView::EdgeLabel { doc, er } => Outcome::applied(
                doc.set_edge_label_color(er, edge_color_as_override(&c).as_deref()),
            ),
            TargetView::PortalLabel {
                doc,
                er,
                endpoint_node_id,
            } => Outcome::applied(doc.set_portal_label_color(
                er,
                endpoint_node_id,
                edge_color_as_override(&c).as_deref(),
            )),
            TargetView::PortalText {
                doc,
                er,
                endpoint_node_id,
            } => Outcome::applied(doc.set_portal_label_text_color(
                er,
                endpoint_node_id,
                edge_color_as_override(&c).as_deref(),
            )),
        }
    }
}

impl<'a> HasFontSize for TargetView<'a> {
    fn set_font_size(&mut self, pt: f32) -> Outcome {
        if !(pt > 0.0) {
            return Outcome::Invalid(format!("must be positive; got {pt}"));
        }
        match self {
            TargetView::Node { doc, id } => Outcome::applied(doc.set_node_font_size(id, pt)),
            TargetView::Edge { doc, er } => Outcome::applied(doc.set_edge_font_size(er, pt)),
            // Sub-parts (label / portal icon / portal text) fall
            // back to the owning-edge font size for now —
            // independent font-size setters on the per-sub-part
            // channels land with the `font size= min= max=`
            // atomic-clamp setter in a follow-up commit.
            TargetView::EdgeLabel { doc, er } => {
                Outcome::applied(doc.set_edge_font_size(er, pt))
            }
            TargetView::PortalLabel { doc, er, .. } => {
                Outcome::applied(doc.set_edge_font_size(er, pt))
            }
            TargetView::PortalText { doc, er, .. } => {
                Outcome::applied(doc.set_edge_font_size(er, pt))
            }
        }
    }
}

impl<'a> AcceptsWheelColor for TargetView<'a> {
    fn apply_wheel_color(&mut self, c: ColorValue) -> Outcome {
        match self {
            // Node default: background fill.
            TargetView::Node { .. } => self.set_bg_color(c),
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
            // Node copy = the node's current text. Empty text reports
            // `Empty` so the caller can distinguish from a target type
            // that doesn't support copy at all.
            TargetView::Node { doc, id } => match doc.mindmap.nodes.get(id) {
                Some(n) if n.text.is_empty() => ClipboardContent::Empty,
                Some(n) => ClipboardContent::Text(n.text.clone()),
                None => ClipboardContent::NotApplicable,
            },
            // Edge copy = the resolved edge color hex. User-facing
            // spec: clipboard copy on an edge copies its colour
            // (changed from the prior label-text behaviour — edge
            // label text is edited through the inline modal, which
            // handles its own OS-clipboard surface).
            TargetView::Edge { doc, er } => {
                let resolved = {
                    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e));
                    edge.map(|e| {
                        let cfg = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
                            e,
                            &doc.mindmap.canvas,
                        );
                        let raw = cfg.color.as_deref().unwrap_or(e.color.as_str());
                        baumhard::util::color::resolve_var(
                            raw,
                            &doc.mindmap.canvas.theme_variables,
                        )
                        .to_string()
                    })
                };
                match resolved {
                    Some(hex) => ClipboardContent::Text(hex),
                    None => ClipboardContent::NotApplicable,
                }
            }
            // Edge label copy = resolved label color hex (cascade:
            // label_config.color → glyph_connection.color →
            // edge.color). Always a concrete hex when the edge
            // exists, so pasting to another target produces a
            // real value.
            TargetView::EdgeLabel { doc, er } => {
                match doc.resolve_edge_label_color(er) {
                    Some(hex) => ClipboardContent::Text(hex),
                    None => ClipboardContent::NotApplicable,
                }
            }
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
            // Paste replaces the node's text with the clipboard
            // contents wholesale. Mirrors the user's mental model:
            // "paste here" = "put this where I'm pointing". Trims
            // trailing whitespace newline noise (a common source
            // when clipboard contents come from a paragraph).
            TargetView::Node { doc, id } => {
                Outcome::applied(doc.set_node_text(id, content.trim_end().to_string()))
            }
            // Edge paste = set edge color from hex (changed from
            // prior label-text behaviour). Invalid contents
            // surface as `Outcome::Invalid` so the user notices a
            // bad paste rather than silently losing a colour edit.
            TargetView::Edge { doc, er } => {
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    return Outcome::applied(doc.set_edge_color(er, None));
                }
                if !is_valid_color_literal(trimmed) {
                    return Outcome::Invalid(format!("not a color: {trimmed}"));
                }
                Outcome::applied(doc.set_edge_color(er, Some(trimmed)))
            }
            // Edge label paste = set the label color override from
            // hex (independent from the edge color, so pasting a
            // hex onto a selected label recolours only the label).
            TargetView::EdgeLabel { doc, er } => {
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    return Outcome::applied(doc.set_edge_label_color(er, None));
                }
                if !is_valid_color_literal(trimmed) {
                    return Outcome::Invalid(format!("not a color: {trimmed}"));
                }
                Outcome::applied(doc.set_edge_label_color(er, Some(trimmed)))
            }
            // Portal icon paste = per-endpoint icon color from hex.
            TargetView::PortalLabel {
                doc,
                er,
                endpoint_node_id,
            } => {
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    return Outcome::applied(
                        doc.set_portal_label_color(er, endpoint_node_id, None),
                    );
                }
                if !is_valid_color_literal(trimmed) {
                    return Outcome::Invalid(format!("not a color: {trimmed}"));
                }
                Outcome::applied(
                    doc.set_portal_label_color(er, endpoint_node_id, Some(trimmed)),
                )
            }
            // Portal text paste = per-endpoint text color from hex.
            TargetView::PortalText {
                doc,
                er,
                endpoint_node_id,
            } => {
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    return Outcome::applied(
                        doc.set_portal_label_text_color(er, endpoint_node_id, None),
                    );
                }
                if !is_valid_color_literal(trimmed) {
                    return Outcome::Invalid(format!("not a color: {trimmed}"));
                }
                Outcome::applied(
                    doc.set_portal_label_text_color(er, endpoint_node_id, Some(trimmed)),
                )
            }
        }
    }
}

impl<'a> HandlesCut for TargetView<'a> {
    fn clipboard_cut(&mut self) -> ClipboardContent {
        match self {
            TargetView::Node { doc, id } => {
                let text = match doc.mindmap.nodes.get(id) {
                    Some(n) => n.text.clone(),
                    None => return ClipboardContent::NotApplicable,
                };
                doc.set_node_text(id, String::new());
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
                let hex = {
                    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e));
                    edge.map(|e| {
                        let cfg = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
                            e,
                            &doc.mindmap.canvas,
                        );
                        let raw = cfg.color.as_deref().unwrap_or(e.color.as_str());
                        baumhard::util::color::resolve_var(
                            raw,
                            &doc.mindmap.canvas.theme_variables,
                        )
                        .to_string()
                    })
                };
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
/// model accepts: `#rrggbb` / `#rrggbbaa` hex codes and
/// `var(--name)` theme references. Keeps the paste path from
/// writing arbitrary strings into the color field — anything else
/// the user might paste (prose, a URL, a number) should surface
/// as `Outcome::Invalid` instead of a corrupt model value.
fn is_valid_color_literal(s: &str) -> bool {
    if let Some(rest) = s.strip_prefix('#') {
        return (rest.len() == 6 || rest.len() == 8)
            && rest.chars().all(|c| c.is_ascii_hexdigit());
    }
    s.starts_with("var(--") && s.ends_with(')')
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
        TargetId::Edge(er) => TargetView::Edge { doc, er: er.clone() },
        TargetId::EdgeLabel(er) => TargetView::EdgeLabel { doc, er: er.clone() },
        TargetId::PortalLabel { edge, endpoint_node_id } => TargetView::PortalLabel {
            doc,
            er: edge.clone(),
            endpoint_node_id: endpoint_node_id.clone(),
        },
        TargetId::PortalText { edge, endpoint_node_id } => TargetView::PortalText {
            doc,
            er: edge.clone(),
            endpoint_node_id: endpoint_node_id.clone(),
        },
    }
}
