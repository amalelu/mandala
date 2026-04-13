//! Capability traits and the [`TargetView`] dispatcher — the core of
//! the console's trait-dispatched cross-cutting command layer.
//!
//! The idea: a command like `color bg=accent text=#fff` doesn't know
//! whether the current selection is a node, an edge, or a portal. It
//! materializes a `Vec<TargetView>` from [`SelectionState`] (a single
//! node, or five multi-selected nodes, or one edge, or one portal)
//! and for each kv-pair calls the corresponding trait method on every
//! target. The target variant that doesn't implement the trait
//! returns [`Outcome::NotApplicable`]; the dispatcher aggregates the
//! outcomes into a single per-kv report.
//!
//! Why `enum TargetView` and not `Box<dyn Trait>`: the set of targets
//! is closed (Node / Edge / Portal) and small, `match` is trivially
//! cheap, and avoiding dynamic dispatch keeps the signatures shorter
//! (no `dyn HasBgColor`). The same principle is in use across the
//! baumhard crate for the mindmap model.

use super::constants::{PORTAL_DEFAULT_COLOR, VAR_ACCENT, VAR_EDGE, VAR_FG};
use crate::application::document::{EdgeRef, MindMapDocument, PortalRef, SelectionState};

/// A parsed color value from the kv-form console. Resolution of
/// `Var` happens downstream at scene-build time via
/// `baumhard::util::color::resolve_var` — the model's color fields
/// are `String`, so passing a structured `ColorValue` through the
/// traits lets each trait impl decide what `Reset` means without
/// pre-resolving `var(--accent)`.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorValue {
    /// Literal hex string, e.g. `"#009c15"` or `"#009c15ff"`. Stored
    /// verbatim so it lands directly in the model's color field.
    Hex(String),
    /// Theme variable reference, e.g. `VAR_ACCENT`. Written into the
    /// model as `"var(--accent)"` so the scene builder resolves it
    /// against the canvas's theme_variables.
    Var(&'static str),
    /// Clear any local override — the component falls back to its
    /// natural default. What "natural default" means is per-trait.
    Reset,
}

impl ColorValue {
    /// Parse `s` into a ColorValue. Accepts:
    /// - `"#rrggbb"` / `"#rrggbbaa"` / `"#rgb"` / `"#rgba"` — as hex
    /// - `"accent"` / `"edge"` / `"fg"` / `"bg"` — as well-known vars
    /// - `"reset"` — as Reset
    ///
    /// Returns `Err(msg)` on unrecognised input. Callers report the
    /// error through the per-kv outcome.
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        if let Some(rest) = t.strip_prefix('#') {
            // Shape-check: 3 / 4 / 6 / 8 hex digits.
            let valid = matches!(rest.len(), 3 | 4 | 6 | 8)
                && rest.chars().all(|c| c.is_ascii_hexdigit());
            if !valid {
                return Err(format!("invalid hex color: '{}'", s));
            }
            return Ok(ColorValue::Hex(t.to_string()));
        }
        match t.to_ascii_lowercase().as_str() {
            "accent" => Ok(ColorValue::Var(VAR_ACCENT)),
            "edge" => Ok(ColorValue::Var(VAR_EDGE)),
            "fg" => Ok(ColorValue::Var(VAR_FG)),
            "reset" => Ok(ColorValue::Reset),
            other => Err(format!("unknown color '{}'", other)),
        }
    }

    /// Render as the string form the model stores — `"#rrggbb"` for
    /// `Hex`, `"var(--...)"` for `Var`. `Reset` has no single string
    /// form and the caller must decide what it means; this method
    /// is only meaningful on `Hex` and `Var`.
    pub fn as_model_string(&self) -> Option<String> {
        match self {
            ColorValue::Hex(s) => Some(s.clone()),
            ColorValue::Var(v) => Some((*v).to_string()),
            ColorValue::Reset => None,
        }
    }
}

/// Outcome of a single trait call. Aggregated by the dispatcher into
/// a per-kv report line.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The setter ran and actually changed something.
    Applied,
    /// The setter ran but the value matched the current state, so
    /// nothing changed. Not an error — distinguishable from
    /// `Applied` so "already set" feedback is possible.
    Unchanged,
    /// The target doesn't implement this trait (e.g. `text=` on a
    /// portal). The dispatcher reports this per-pair so `color
    /// bg=#fff text=accent` applies the `bg` to a portal while the
    /// `text` pair is reported as not-applicable.
    NotApplicable,
    /// The value was rejected by the target (e.g. a negative font
    /// size).
    Invalid(String),
}

impl Outcome {
    pub fn applied(changed: bool) -> Self {
        if changed { Outcome::Applied } else { Outcome::Unchanged }
    }
}

// --- capability traits -----------------------------------------------

/// Target supports setting its background / fill color.
///
/// For nodes this is the frame fill. For portals it's the glyph
/// color itself (portals have no separate fill). For edges it is
/// unsupported — edges don't have a fill concept.
pub trait HasBgColor {
    fn set_bg_color(&mut self, c: ColorValue) -> Outcome;
}

/// Target supports setting its foreground / text color.
///
/// For nodes this rewrites `style.text_color` and any `TextRun`
/// whose color matched the pre-edit default (per-run overrides are
/// preserved). For edges this is the label / line color. For
/// portals it is unsupported.
pub trait HasTextColor {
    fn set_text_color(&mut self, c: ColorValue) -> Outcome;
}

/// Target supports setting its border / outline color.
///
/// For nodes this is `style.frame_color`. For edges this is the
/// connection body (glyph) color. For portals it is unsupported.
pub trait HasBorderColor {
    fn set_border_color(&mut self, c: ColorValue) -> Outcome;
}

/// Target supports setting its font size in points.
pub trait HasFontSize {
    fn set_font_size(&mut self, pt: f32) -> Outcome;
}

/// Target supports setting or clearing a label.
///
/// `None` clears the label; `Some(s)` sets it. For edges this is
/// `MindEdge.label`; nodes and portals do not implement it.
pub trait HasLabel {
    fn set_label(&mut self, s: Option<String>) -> Outcome;
}

/// Target supports receiving a color from the **standalone color
/// wheel** (the `color picker on` persistent palette). The wheel
/// doesn't pick an axis — it pushes one color at the selection and
/// asks each component type to decide which channel that color
/// belongs on. Nodes take it on `Bg`; edges take it on their single
/// color field (routed through `set_border_color`, which is the same
/// sink as `set_text_color` for edges). Portals haven't been ported
/// to Baumhard yet — they return `NotApplicable` today and will
/// switch to their fill channel once the port lands.
///
/// Separate trait from `HasBgColor` / `HasTextColor` / `HasBorderColor`
/// by design: the `Has*` axis traits answer "can you accept a color
/// on channel X?"; `AcceptsWheelColor` answers the narrower question
/// "if someone hands you one color without specifying a channel,
/// where does it go?". The default-channel choice belongs with the
/// component implementation, not with every caller.
pub trait AcceptsWheelColor {
    fn apply_wheel_color(&mut self, c: ColorValue) -> Outcome;
}

// --- target view -----------------------------------------------------

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

// --- materialization -------------------------------------------------

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

// --- dispatch ---------------------------------------------------------

/// Formatted summary of a command's per-kv outcome across targets,
/// used to render the scrollback line. Returns `Ok`-style text on
/// success; if at least one pair failed validation the caller turns
/// it into an `ExecResult::Err`.
pub struct DispatchReport {
    /// Count of pairs that at least one target accepted with a
    /// change. Used to pick "set" vs "unchanged" phrasing.
    pub any_applied: bool,
    /// Messages to print to scrollback, one per issue. Empty when
    /// everything applied cleanly.
    pub messages: Vec<String>,
    /// True if every pair was either Invalid or had no applicable
    /// target — `execute` then wants to turn the report into an Err.
    pub all_failed: bool,
}

/// Apply a list of kv-pairs to a TargetView list, dispatching each
/// key through the corresponding trait. `applier` tells the
/// dispatcher what trait a given key maps to and how to invoke it.
///
/// `applier` returns:
/// - `Some(Outcome)` — the key is recognized; the outcome was the
///   result of the trait call on this target
/// - `None` — the key is not recognized at all (e.g. `font bogus=1`);
///   the dispatcher reports it once (not once per target)
pub fn apply_kvs<F>(
    doc: &mut MindMapDocument,
    kvs: &[(String, String)],
    mut applier: F,
) -> DispatchReport
where
    F: FnMut(&mut TargetView, &str, &str) -> Option<Outcome>,
{
    let targets = selection_targets(&doc.selection);
    if targets.is_empty() {
        return DispatchReport {
            any_applied: false,
            messages: vec!["no target for command (select a node, edge, or portal first)".into()],
            all_failed: true,
        };
    }

    let mut any_applied = false;
    let mut messages: Vec<String> = Vec::new();
    let mut any_pair_succeeded = false;

    for (k, v) in kvs {
        // Aggregate this pair across every target.
        let mut applied_count = 0usize;
        let mut unchanged_count = 0usize;
        let mut na_count = 0usize;
        let mut invalid_msgs: Vec<String> = Vec::new();
        let mut unknown_key = false;

        for tid in &targets {
            let mut view = view_for(doc, tid);
            match applier(&mut view, k, v) {
                Some(Outcome::Applied) => {
                    applied_count += 1;
                }
                Some(Outcome::Unchanged) => {
                    unchanged_count += 1;
                }
                Some(Outcome::NotApplicable) => {
                    na_count += 1;
                }
                Some(Outcome::Invalid(msg)) => {
                    invalid_msgs.push(msg);
                }
                None => {
                    unknown_key = true;
                    break;
                }
            }
        }

        if unknown_key {
            messages.push(format!("unknown key '{}'", k));
            continue;
        }
        if !invalid_msgs.is_empty() {
            for m in invalid_msgs {
                messages.push(format!("{}: {}", k, m));
            }
            continue;
        }
        if applied_count > 0 {
            any_applied = true;
            any_pair_succeeded = true;
        } else if unchanged_count > 0 {
            any_pair_succeeded = true;
            messages.push(format!("{} already {}", k, v));
        } else if na_count == targets.len() {
            messages.push(format!(
                "{}: not applicable to {}",
                k,
                targets_kind_label(&targets),
            ));
        }
    }

    let all_failed = !any_pair_succeeded && !messages.is_empty();
    DispatchReport {
        any_applied,
        messages,
        all_failed,
    }
}

fn targets_kind_label(targets: &[TargetId]) -> &'static str {
    // Multi-selection is homogeneously nodes today; other combos
    // are single-target. Pick the obvious label.
    match targets.first() {
        Some(TargetId::Node(_)) => {
            if targets.len() > 1 {
                "nodes"
            } else {
                "node"
            }
        }
        Some(TargetId::Edge(_)) => "edge",
        Some(TargetId::Portal(_)) => "portal",
        None => "selection",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_ok() {
        assert_eq!(ColorValue::parse("#123").unwrap(), ColorValue::Hex("#123".into()));
        assert_eq!(
            ColorValue::parse("#009c15").unwrap(),
            ColorValue::Hex("#009c15".into())
        );
        assert_eq!(
            ColorValue::parse("#009c15ff").unwrap(),
            ColorValue::Hex("#009c15ff".into())
        );
    }

    #[test]
    fn test_parse_hex_rejects_bad_length() {
        assert!(ColorValue::parse("#12").is_err());
        assert!(ColorValue::parse("#12345").is_err());
        assert!(ColorValue::parse("#zzzzzz").is_err());
    }

    #[test]
    fn test_parse_var_tokens() {
        assert_eq!(ColorValue::parse("accent").unwrap(), ColorValue::Var(VAR_ACCENT));
        assert_eq!(ColorValue::parse("ACCENT").unwrap(), ColorValue::Var(VAR_ACCENT));
        assert_eq!(ColorValue::parse("fg").unwrap(), ColorValue::Var(VAR_FG));
        assert_eq!(ColorValue::parse("edge").unwrap(), ColorValue::Var(VAR_EDGE));
    }

    #[test]
    fn test_parse_reset() {
        assert_eq!(ColorValue::parse("reset").unwrap(), ColorValue::Reset);
    }

    #[test]
    fn test_parse_unknown_is_error() {
        assert!(ColorValue::parse("bogus").is_err());
    }

    #[test]
    fn test_outcome_applied_helper() {
        assert_eq!(Outcome::applied(true), Outcome::Applied);
        assert_eq!(Outcome::applied(false), Outcome::Unchanged);
    }

    #[test]
    fn test_selection_targets_for_each_variant() {
        use crate::application::document::{EdgeRef, PortalRef};
        assert!(selection_targets(&SelectionState::None).is_empty());

        let ids = vec!["a".to_string(), "b".to_string()];
        let out = selection_targets(&SelectionState::Multi(ids.clone()));
        assert_eq!(out.len(), 2);

        let er = EdgeRef::new("a", "b", "cross_link");
        let out = selection_targets(&SelectionState::Edge(er));
        assert!(matches!(out.as_slice(), [TargetId::Edge(_)]));

        let pr = PortalRef {
            label: "A".into(),
            endpoint_a: "x".into(),
            endpoint_b: "y".into(),
        };
        let out = selection_targets(&SelectionState::Portal(pr));
        assert!(matches!(out.as_slice(), [TargetId::Portal(_)]));
    }

}
