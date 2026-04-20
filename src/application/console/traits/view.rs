//! `TargetView` — the enum that holds a mutable doc reference plus
//! enough identity to find the component each iteration. All
//! capability-trait impls live here; selection materialization
//! (`selection_targets`, `view_for`) sits with the view since each
//! is a single-line constructor.
//!
//! Post-refactor there are two target shapes: `Node` and `Edge`.
//! Portal-mode edges go through the `Edge` shape just like
//! line-mode edges — `display_mode` is a render flag, not a
//! separate entity, so trait dispatch doesn't split on it.

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
    Edge { doc: &'a mut MindMapDocument, er: EdgeRef },
    /// One endpoint's portal label on a portal-mode edge. Carries
    /// both the owning-edge ref and the endpoint-node id so the
    /// trait arms can route mutations to the correct
    /// `PortalEndpointState`. Wheel / copy / paste / cut all
    /// operate on the label's **color**: portal labels are a
    /// single-glyph badge, so there's no separate text / font /
    /// border concept — color is the only thing a user would
    /// ever target here.
    PortalLabel {
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
            TargetView::PortalLabel { .. } => "portal label",
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
            TargetView::Edge { .. } => Outcome::NotApplicable,
            TargetView::PortalLabel { .. } => Outcome::NotApplicable,
        }
    }
}

impl<'a> HasTextColor for TargetView<'a> {
    fn set_text_color(&mut self, c: ColorValue) -> Outcome {
        match self {
            TargetView::Node { doc, id } => {
                Outcome::applied(doc.set_node_text_color(id, color_as_string(&c, "#ffffff")))
            }
            TargetView::Edge { doc, er } => Outcome::applied(
                doc.set_edge_color(er, edge_color_as_override(&c).as_deref()),
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
        }
    }
}

impl<'a> HasBorderColor for TargetView<'a> {
    fn set_border_color(&mut self, c: ColorValue) -> Outcome {
        match self {
            TargetView::Node { doc, id } => Outcome::applied(
                doc.set_node_border_color(id, color_as_string(&c, "#ffffff")),
            ),
            TargetView::Edge { doc, er } => Outcome::applied(
                doc.set_edge_color(er, edge_color_as_override(&c).as_deref()),
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
            // Portal labels inherit the edge's font size. Editing
            // font on a portal label routes through the owning edge
            // so both markers stay in sync — independent per-label
            // sizing isn't a user-level concern.
            TargetView::PortalLabel { doc, er, .. } => {
                Outcome::applied(doc.set_edge_font_size(er, pt))
            }
        }
    }
}

impl<'a> AcceptsWheelColor for TargetView<'a> {
    fn apply_wheel_color(&mut self, c: ColorValue) -> Outcome {
        match self {
            // Node default: background fill. Matches the `color bg`
            // console verb when no axis is specified, and reads as
            // "paint the node" visually — the bg is the dominant
            // surface of a node glyph-frame, so colouring it is the
            // most noticeable response to a wheel commit.
            TargetView::Node { .. } => self.set_bg_color(c),
            // Edge default: the one color field, routed through
            // `set_border_color` (which is the same sink as
            // `set_text_color` on edges — `MindEdge.color` drives
            // both the line and the label; there is no separate
            // bg / text / border on an edge). Works identically
            // for portal-mode edges, where the same color drives
            // the two marker glyphs.
            TargetView::Edge { .. } => self.set_border_color(c),
            // Portal label default: the per-endpoint color
            // override. Routed through `set_border_color` (which
            // delegates to `set_portal_label_color` on this
            // variant) for consistency with the edge wheel path.
            TargetView::PortalLabel { .. } => self.set_border_color(c),
        }
    }
}

impl<'a> HasLabel for TargetView<'a> {
    fn set_label(&mut self, s: Option<String>) -> Outcome {
        match self {
            TargetView::Edge { doc, er } => Outcome::applied(doc.set_edge_label(er, s)),
            // Portal label → per-endpoint text. Shares the
            // `label text="…"` console syntax with edges; the
            // routing splits here based on the selection kind.
            TargetView::PortalLabel {
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
            // Edge copy = the edge's current label, if any. Missing
            // / empty labels report `Empty`.
            TargetView::Edge { doc, er } => match read_edge_label(doc, er) {
                Some(t) if t.is_empty() => ClipboardContent::Empty,
                Some(t) => ClipboardContent::Text(t),
                None => ClipboardContent::Empty,
            },
            // Portal label copy = the resolved hex color. Always a
            // real value (cascade fallback resolves to a concrete
            // hex even when no override is set), so the user can
            // always paste it elsewhere. Reported as `NotApplicable`
            // rather than `Empty` when the edge disappears mid-op.
            TargetView::PortalLabel {
                doc,
                er,
                endpoint_node_id,
            } => match doc.resolve_portal_label_color(er, endpoint_node_id) {
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
            // Paste sets the edge label to the clipboard contents.
            // Empty content clears the label (Some("") would also
            // clear, but None is the canonical "clear" form).
            TargetView::Edge { doc, er } => {
                let trimmed = content.trim_end();
                let label = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
                Outcome::applied(doc.set_edge_label(er, label))
            }
            // Portal label paste = set the per-endpoint color from
            // the clipboard contents. Accepts `#rrggbb` and
            // `var(--name)` forms, trimmed of whitespace. Empty /
            // invalid content is reported as `Invalid` rather than
            // silently ignored, so the user notices a bad paste.
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
            TargetView::Edge { doc, er } => {
                let label = read_edge_label(doc, er);
                doc.set_edge_label(er, None);
                match label {
                    Some(t) if !t.is_empty() => ClipboardContent::Text(t),
                    _ => ClipboardContent::Empty,
                }
            }
            // Portal label cut = copy the resolved hex, then clear
            // the per-endpoint override so the label reverts to
            // inheriting from the edge. The user still gets a real
            // hex in the clipboard (the fallback cascade computed
            // one), but the label visually resets to the edge color.
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
    PortalLabel {
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
        SelectionState::PortalLabel(s) => vec![TargetId::PortalLabel {
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
        TargetId::PortalLabel { edge, endpoint_node_id } => TargetView::PortalLabel {
            doc,
            er: edge.clone(),
            endpoint_node_id: endpoint_node_id.clone(),
        },
    }
}
