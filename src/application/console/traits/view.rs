//! `TargetView` — the enum that holds a mutable doc reference plus
//! enough identity to find the component each iteration. All
//! capability-trait impls live here; selection materialization
//! (`selection_targets`, `view_for`) sits with the view since each
//! is a single-line constructor.

use super::capabilities::{
    AcceptsWheelColor, HasBgColor, HasBorderColor, HasFontSize, HandlesCopy, HandlesCut,
    HandlesPaste, HasLabel, HasTextColor,
};
use super::color_value::ColorValue;
use super::outcome::{ClipboardContent, Outcome};
use crate::application::console::constants::PORTAL_DEFAULT_COLOR;
use crate::application::document::{EdgeRef, MindMapDocument, PortalRef, SelectionState};

/// A mutable view into one selected component, holding the doc ref
/// plus enough identity to find the component each time. Built fresh
/// per-iteration in a `Multi` fanout so no two views hold aliasing
/// `&mut doc` borrows at once.
pub enum TargetView<'a> {
    Node { doc: &'a mut MindMapDocument, id: String },
    Edge { doc: &'a mut MindMapDocument, er: EdgeRef },
    Portal { doc: &'a mut MindMapDocument, pr: PortalRef },
}

impl<'a> TargetView<'a> {
    /// One-word label, used in per-target error messages.
    pub fn kind(&self) -> &'static str {
        match self {
            TargetView::Node { .. } => "node",
            TargetView::Edge { .. } => "edge",
            TargetView::Portal { .. } => "portal",
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
            TargetView::Portal { doc, pr } => {
                let color = color_as_string(&c, PORTAL_DEFAULT_COLOR);
                Outcome::applied(doc.set_portal_color(pr, &color))
            }
            TargetView::Edge { .. } => Outcome::NotApplicable,
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
            TargetView::Portal { .. } => Outcome::NotApplicable,
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
            // Portals don't have a separate border from their fill.
            TargetView::Portal { .. } => Outcome::NotApplicable,
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
            TargetView::Portal { .. } => Outcome::NotApplicable,
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
            // bg / text / border on an edge). Picking `border` as
            // the name reads more honestly than `text`: the edge
            // *is* the line.
            TargetView::Edge { .. } => self.set_border_color(c),
            // Portals aren't Baumhard-native yet — holding on this
            // arm until they are. When portals migrate, this
            // becomes `self.set_bg_color(c)` (portals have a single
            // color field that behaves as a fill).
            TargetView::Portal { .. } => Outcome::NotApplicable,
        }
    }
}

impl<'a> HasLabel for TargetView<'a> {
    fn set_label(&mut self, s: Option<String>) -> Outcome {
        match self {
            TargetView::Edge { doc, er } => Outcome::applied(doc.set_edge_label(er, s)),
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
            // Portal copy = the portal's current color string (a hex
            // or `var(--…)` reference). Always present in the model.
            TargetView::Portal { doc, pr } => match read_portal_color(doc, pr) {
                Some(c) => ClipboardContent::Text(c),
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
            // Paste sets the portal color, but only after validating
            // the input is a `#RRGGBB` hex or a `var(--name)` ref.
            // Non-color text would otherwise silently corrupt the
            // model (the renderer falls back to default but the
            // saved file would carry junk).
            TargetView::Portal { doc, pr } => {
                let trimmed = content.trim();
                if !is_color_string(trimmed) {
                    return Outcome::Invalid(format!("not a color: {}", trimmed));
                }
                Outcome::applied(doc.set_portal_color(pr, trimmed))
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
            // Portal cut returns the color and resets to the
            // PORTAL_DEFAULT_COLOR — there's no "no color" state
            // (the field is required), so cut means "back to default".
            TargetView::Portal { doc, pr } => {
                let color = match read_portal_color(doc, pr) {
                    Some(c) => c,
                    None => return ClipboardContent::NotApplicable,
                };
                doc.set_portal_color(pr, PORTAL_DEFAULT_COLOR);
                ClipboardContent::Text(color)
            }
        }
    }
}

fn read_edge_label(doc: &MindMapDocument, er: &EdgeRef) -> Option<String> {
    let idx = doc.edge_index(er)?;
    doc.mindmap.edges.get(idx).and_then(|e| e.label.clone())
}

fn read_portal_color(doc: &MindMapDocument, pr: &PortalRef) -> Option<String> {
    doc.mindmap
        .portals
        .iter()
        .find(|p| pr.matches(p))
        .map(|p| p.color.clone())
}

/// Accept `#RGB`, `#RRGGBB`, `#RRGGBBAA`, or `var(--name)`. Conservative
/// — anything else round-trips through `Outcome::Invalid` rather than
/// being silently written to the model.
fn is_color_string(s: &str) -> bool {
    if s.starts_with("var(--") && s.ends_with(')') {
        return true;
    }
    if !s.starts_with('#') {
        return false;
    }
    let hex = &s[1..];
    matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// Snapshot the selection into a list of target identities the
/// dispatcher can iterate over. Returns owned strings / refs so the
/// caller can build a fresh `TargetView` per iteration (aliasing-
/// safe fanout).
pub enum TargetId {
    Node(String),
    Edge(EdgeRef),
    Portal(PortalRef),
}

pub fn selection_targets(sel: &SelectionState) -> Vec<TargetId> {
    match sel {
        SelectionState::None => Vec::new(),
        SelectionState::Single(id) => vec![TargetId::Node(id.clone())],
        SelectionState::Multi(ids) => ids.iter().cloned().map(TargetId::Node).collect(),
        SelectionState::Edge(er) => vec![TargetId::Edge(er.clone())],
        SelectionState::Portal(pr) => vec![TargetId::Portal(pr.clone())],
    }
}

/// Rebuild a `TargetView` on a fresh `&mut doc` borrow. Call this
/// once per iteration of the fanout loop so no two views overlap.
pub fn view_for<'a>(doc: &'a mut MindMapDocument, id: &TargetId) -> TargetView<'a> {
    match id {
        TargetId::Node(nid) => TargetView::Node { doc, id: nid.clone() },
        TargetId::Edge(er) => TargetView::Edge { doc, er: er.clone() },
        TargetId::Portal(pr) => TargetView::Portal { doc, pr: pr.clone() },
    }
}
