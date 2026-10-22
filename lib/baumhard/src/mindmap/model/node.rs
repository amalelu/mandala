// SPDX-License-Identifier: MPL-2.0

//! Node data model: `MindNode` and the small structs that travel with
//! it — position, size, text runs, node style, layout, color schema,
//! and the glyph-border config. Borders belong here because they are
//! always per-node (no edge-level borders exist).

use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::gfx_structs::zoom_visibility::ZoomVisibility;
use crate::mindmap::custom_mutation::{CustomMutation, TriggerBinding};

/// Absolute ceiling on `MindNode.size.{width,height}` in canvas units.
/// Shared by the app's node-size setters and `maptool verify` so a
/// hand-edited map cannot pass verify with a size the interactive
/// editor would refuse to produce. The value is large enough to
/// swallow any realistic canvas extent and small enough to flag an
/// extra zero or two as the canonical typo.
pub const MAX_NODE_AXIS: f64 = 1_000_000.0;

/// Hard cap on `MindNode.sections.len()`, enforced **while
/// deserializing** so the refusal costs nothing.
///
/// It is a bound on one node's content strata, not on map size: a
/// large map has many nodes, not one node with millions of sections,
/// so this does not stand between the reader and a big document.
///
/// **The enforcement point is the whole of it.** This constant existed
/// with a doc claiming it "defends against hostile mindmaps with
/// `sections: [{},{},…10M…]` that would OOM on load", and it did not:
/// the check lived in the loader's invariant sweep, which runs after
/// serde has built the whole `Vec`. A 12 MB file declaring 4 000 000
/// sections reached **1 723 MiB resident** and was then refused — the
/// message arriving after the cost it was written to avoid, and a file
/// ten times larger would have died before the check ran at all.
///
/// `deserialize_sections_capped` stops at the first element past the
/// cap, so the same file now costs the bytes read and nothing more.
/// (Code span rather than a link: it is a private `fn` and this is a
/// `pub const`, so rustdoc refuses the link under `-D warnings`. Its
/// sibling `detect_section_count_cap` below is spelled the same way.)
/// `detect_section_count_cap` stays as the invariant for maps built in
/// memory rather than parsed, and `maptool verify` still reports it.
pub const MAX_SECTIONS_PER_NODE: usize = 1024;

/// Read an absent-or-`null` value as `T::default()`.
///
/// `#[serde(default)]` covers the *missing key* and nothing else, so
/// a field that is conceptually optional still refuses an explicit
/// `null`. Pair the two on any optional object whose absence and
/// whose `null` mean the same thing.
///
/// Cost: one `Option<T>` deserialize, no allocation of its own.
fn deserialize_null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// Deserialize `sections`, refusing at the first element past
/// [`MAX_SECTIONS_PER_NODE`] rather than after the allocation.
///
/// Counts as it goes instead of trusting `size_hint`, which a hostile
/// document controls and which serde documents as a hint rather than a
/// guarantee.
fn deserialize_sections_capped<'de, D>(deserializer: D) -> Result<Vec<MindSection>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Error, SeqAccess, Visitor};
    use std::fmt;

    struct Capped;

    impl<'de> Visitor<'de> for Capped {
        type Value = Vec<MindSection>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "at most {MAX_SECTIONS_PER_NODE} sections")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            // Not `with_capacity(size_hint)` — the hint comes from the
            // document, so honoring it would hand the allocation back
            // to the attacker this exists to stop.
            let mut out: Vec<MindSection> = Vec::new();
            while let Some(section) = seq.next_element::<MindSection>()? {
                if out.len() >= MAX_SECTIONS_PER_NODE {
                    return Err(A::Error::custom(format!(
                        "node.sections.len() exceeds cap {MAX_SECTIONS_PER_NODE} — every \
                         renderable node needs at least one section and no authoring case \
                         approaches this many"
                    )));
                }
                out.push(section);
            }
            Ok(out)
        }
    }

    deserializer.deserialize_seq(Capped)
}

/// A single node in the mindmap: a styled rectangle that hosts one
/// or more [`MindSection`]s carrying the text content. The node
/// itself owns the visual chrome (background, frame, shape, border,
/// shadow) and the structural pieces (`parent_id`, `channel`,
/// trigger bindings, palette binding); the *user-facing strata of
/// data* live on its sections.
///
/// Attached to a tree position via [`Self::parent_id`] and a
/// Dewey-decimal [`Self::id`]. The loader materializes one of these
/// per `.mindmap.json` entry; the scene builder and tree builder
/// both project from this shape.
///
/// Plain data; no runtime cost beyond the `String` allocations serde
/// performs on deserialize.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MindNode {
    /// Dewey-decimal node id (e.g. `"0"`, `"0.1"`, `"0.1.3"`); the
    /// parent prefix establishes the tree structure redundantly with
    /// [`Self::parent_id`]. Must be unique across the map.
    pub id: String,
    /// Parent node id, or `None` for the root. Source of truth for
    /// tree structure; [`Self::id`]'s Dewey prefix must agree.
    pub parent_id: Option<String>,
    /// Canvas-space top-left corner of the node's AABB.
    pub position: Position,
    /// Canvas-space width and height of the node's AABB.
    pub size: Size,
    /// The user data strata of this node, in render order. Each
    /// section is a positioned text-bearing surface inside the node
    /// AABB. Validation invariants:
    ///
    /// - `sections` is non-empty for every renderable node — the
    ///   loader rejects maps where any node ships zero sections,
    ///   pointing at `maptool convert --sections` for migration.
    /// - Section `offset` + `size` should stay within the node's
    ///   AABB; `maptool verify` flags out-of-bounds sections.
    /// - Section `channel` collisions with sibling sections are
    ///   warned (not failed) by `verify`; collisions broadcast a
    ///   single mutation across both, which is occasionally the
    ///   intent.
    ///
    /// `#[serde(default)]` lets the typed shape parse a node
    /// without `sections` (deserializing as empty) so unit tests
    /// that synthesize `MindNode` from raw JSON skipping
    /// section authoring still parse — the *loader* is the layer
    /// that rejects zero-section maps with a migration pointer.
    #[serde(default, deserialize_with = "deserialize_sections_capped")]
    pub sections: Vec<MindSection>,
    /// Background / frame / text colors, border, shape, and the
    /// visible-frame toggle. The text color here acts as the
    /// node-level default — sections without their own per-run
    /// color override fall through to it.
    pub style: NodeStyle,
    /// Layout descriptor carried through from miMind-format source
    /// maps. Mandala drives layout through custom mutations instead
    /// (see `format/mutations.md`); this is round-trip fidelity only.
    pub layout: NodeLayout,
    /// `true` when the node's subtree is collapsed — children stay in
    /// the model but the scene builder treats them as hidden.
    pub folded: bool,
    /// Long-form text attached to the node; rendered separately from
    /// the node's [`MindSection`] text by the notes overlay path.
    /// Carried at the node level rather than per-section because
    /// notes annotate the whole node, not any one stratum of data
    /// inside it.
    pub notes: String,
    /// Optional palette binding that colors this node and its
    /// descendants at a given depth level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_schema: Option<ColorSchema>,
    /// Channel index for mutation targeting in the baumhard tree.
    /// Multiple siblings can share a channel to form broadcast groups.
    #[serde(default)]
    pub channel: usize,
    /// Trigger bindings attached to this specific node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trigger_bindings: Vec<TriggerBinding>,
    /// Inline custom mutations defined on this node (not shared with other nodes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_mutations: Vec<CustomMutation>,
    /// Per-node macro definitions, opaque to baumhard. Loaded into
    /// the application-side `MacroRegistry` at the `Inline` tier
    /// (highest precedence — overrides Map / User / App on
    /// id collision). Stored as untyped JSON values for the same
    /// reason as `MindMap.macros`: the typed `Macro` lives in the
    /// application crate.
    ///
    /// Privilege model: Inline-tier macros — same as Map-tier —
    /// cannot run `ConsoleLine` or destructive `Action` variants
    /// (`SaveDocument`, `DeleteSelection`, `Cut`, `Paste`, `Copy`,
    /// `OrphanSelection`, `CreateOrphanNode`,
    /// `CreateOrphanNodeAndEdit`, `DoubleClickActivate`,
    /// `EditSelection`, `EditSelectionClean`, `NewDocument`).
    /// The privilege gate is enforced at dispatch time in the
    /// application's `dispatch_macro`; see `format/macros.md`
    /// for the full threat model.
    ///
    /// Cross-node id collisions inside the Inline tier are
    /// non-deterministic — namespace your ids
    /// (e.g. `<node-id>.action`) to avoid them. The loader emits
    /// a `warn!` when a collision is detected at load time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_macros: Vec<serde_json::Value>,
    /// Lower bound on `camera.zoom` at which this node (and its
    /// glyph border, which inherits from the node) renders.
    /// `None` = unbounded below. Mirrors the
    /// `min_font_size_pt` / `max_font_size_pt` pair on
    /// [`crate::mindmap::model::edge::GlyphConnectionConfig`] —
    /// same flat-optional posture, orthogonal concept (presence
    /// vs. size). Inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_zoom_to_render: Option<f32>,
    /// Upper bound on `camera.zoom` at which this node renders.
    /// `None` = unbounded above. Inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_zoom_to_render: Option<f32>,
}

impl MindNode {
    /// Move this node to an absolute canvas position, clamped into the
    /// domain the loader accepts.
    ///
    /// **Every write to [`Self::position`] goes through this or
    /// [`Self::offset_position_clamped`]**, and
    /// `test_every_node_position_write_goes_through_the_clamp` fails
    /// the build if a new one does not. That is the shape this bound
    /// needs, because the alternative was tried and did not hold: the
    /// guard began on one writer, grew to two, and a review round found
    /// five more — the interactive drag, two animation writers, and the
    /// custom-mutation sync-back, the last reachable from a map's own
    /// trigger bindings rather than from the editor. Clamping each site
    /// as it is discovered is what produced that history.
    ///
    /// Clamped rather than refused because these are computed metrics,
    /// not authored strings: a layout pass, a drag and an animation
    /// each *derive* a coordinate, and the useful answer to one that
    /// left the canvas is the edge of the canvas. A refusal would have
    /// to be handled at every call site, and the sites that would have
    /// to handle it are exactly the interactive ones
    /// (CODE_CONVENTIONS §9). Authored positions are different and are
    /// **rejected** at the loader and at `set_node_aabb`, which is what
    /// `validate_node_position` is for.
    ///
    /// Non-finite in becomes `0.0`, per
    /// [`clamp_canvas_coord`](crate::mindmap::model::validate::clamp_canvas_coord)
    /// — fully qualified because `validate` is not in scope here, which
    /// is the same path the body below calls.
    pub fn set_position_clamped(&mut self, x: f64, y: f64) {
        self.position.x = crate::mindmap::model::validate::clamp_canvas_coord(x);
        self.position.y = crate::mindmap::model::validate::clamp_canvas_coord(y);
    }

    /// Offset this node's position by a delta, clamped into the same
    /// domain as [`Self::set_position_clamped`].
    ///
    /// Separate from the absolute setter because the drag and animation
    /// writers accumulate: `position += delta` applied repeatedly is
    /// how a coordinate walks out of the domain a step at a time, with
    /// no single step looking wrong. Clamping the *sum* is what stops
    /// it; clamping the delta would not.
    pub fn offset_position_clamped(&mut self, dx: f64, dy: f64) {
        self.set_position_clamped(self.position.x + dx, self.position.y + dy);
    }
}

impl MindNode {
    /// This node's authored zoom window, as a
    /// [`ZoomVisibility`]. O(1).
    ///
    /// # Border inheritance
    ///
    /// Borders inherit this window verbatim via
    /// [`BorderNodeData::zoom_visibility`] in
    /// `tree_builder/border.rs`, stamped onto all eight runs in
    /// `border_node_data`. No separate per-border override
    /// exists today; the floating-frame-fragment case a
    /// non-inheriting border would produce is prevented by
    /// construction. A future
    /// `GlyphBorderConfig.min_zoom_to_render` field would need
    /// to revisit that call site.
    ///
    /// [`BorderNodeData::zoom_visibility`]: crate::mindmap::tree_builder::BorderNodeData
    pub fn zoom_window(&self) -> ZoomVisibility {
        ZoomVisibility::from_pair(self.min_zoom_to_render, self.max_zoom_to_render)
    }

    /// Top-left corner of the node's AABB in canvas space, as a
    /// `glam::Vec2`. The model stores position as `f64` for serde
    /// fidelity; downstream geometry (camera transforms, hit tests,
    /// connection sampling) runs at `f32`. This helper performs the
    /// conversion at the model boundary so call sites can drop the
    /// `Vec2::new(node.position.x as f32, node.position.y as f32)`
    /// boilerplate.
    ///
    /// **Costs.** Two f64→f32 narrowing casts. No allocation
    /// (`glam::Vec2` is `#[repr(C)] [f32; 2]`). Lossy past ~16M
    /// canvas pixels — beyond f32's 24-bit integer-precise range —
    /// but well clear of any realistic mindmap canvas size.
    /// Inlinable; the optimizer folds construction-then-`.x`/`.y`
    /// access into bare register loads.
    pub fn pos_vec2(&self) -> Vec2 {
        Vec2::new(self.position.x as f32, self.position.y as f32)
    }

    /// Width/height of the node's AABB in canvas space, as a
    /// `glam::Vec2`. Sibling of [`Self::pos_vec2`].
    ///
    /// **Costs.** Same as [`Self::pos_vec2`] — two f64→f32 casts,
    /// no allocation, optimizer-foldable.
    pub fn size_vec2(&self) -> Vec2 {
        Vec2::new(self.size.width as f32, self.size.height as f32)
    }

    /// All section text concatenated with `'\n'` between sections —
    /// for legacy consumers (markdown export, plain-text dump) that
    /// want one rendered string per node. Single-section nodes (the
    /// legacy-migration default) round-trip with no surprises:
    /// `display_text()` is just `sections[0].text`.
    ///
    /// **Costs.** O(total section text bytes); one fresh `String`
    /// allocation when the node has 2+ sections. The single-section
    /// fast path borrows `sections[0].text` and returns
    /// `Cow::Borrowed`, so reads on the common case (most nodes
    /// have one section) are zero-allocation. Hot paths that need
    /// per-section rendering should walk `sections` directly
    /// instead of reaching for this helper.
    pub fn display_text(&self) -> std::borrow::Cow<'_, str> {
        match self.sections.len() {
            0 => std::borrow::Cow::Borrowed(""),
            1 => std::borrow::Cow::Borrowed(self.sections[0].text.as_str()),
            _ => {
                let mut out = String::new();
                for (i, section) in self.sections.iter().enumerate() {
                    if i > 0 {
                        out.push('\n');
                    }
                    out.push_str(&section.text);
                }
                std::borrow::Cow::Owned(out)
            }
        }
    }

    /// Center of the node's AABB in canvas space — `pos + size / 2`.
    /// Used by edge-anchor and connection-routing math that needs
    /// the node's geometric center rather than its top-left corner.
    ///
    /// **Costs.** Four f64→f32 casts (two from each helper) plus a
    /// componentwise `Vec2 * f32` and `Vec2 + Vec2`. With glam's
    /// SIMD path enabled the multiply-and-add is two SSE/NEON ops.
    /// Per-frame call sites typically hit this once per visible
    /// edge endpoint; if a profile implicates this hot, add a
    /// `do_*()` benchmark in `lib/baumhard/benches/test_bench.rs`
    /// alongside the existing geometry benches.
    pub fn center_vec2(&self) -> Vec2 {
        self.pos_vec2() + self.size_vec2() * 0.5
    }
}

/// Canvas-space top-left corner of a node's AABB, or — when used on
/// a [`MindSection`] — the node-local offset where the section
/// sits inside its owning node. Units are arbitrary canvas pixels
/// (the camera transforms to screen space at render time). Plain
/// data; no runtime cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Position {
    /// Canvas-space x coordinate (or node-local offset for sections).
    #[serde(default)]
    pub x: f64,
    /// Canvas-space y coordinate (canvas y-axis grows downward).
    #[serde(default)]
    pub y: f64,
}

/// Canvas-space extent of a node's AABB. Width and height are
/// strictly positive in practice but not checked at type level —
/// scene-builder code guards against zero-size nodes on its own.
/// Plain data; no runtime cost.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Size {
    /// Width in canvas units.
    pub width: f64,
    /// Height in canvas units.
    pub height: f64,
}

/// One stratum of user data inside a [`MindNode`]. A section is a
/// positioned, text-bearing surface that lives inside the parent
/// node's AABB. Nodes can carry one or many sections; the
/// post-section-refactor data shape moves the old per-node
/// `text` + `text_runs` pair onto each section.
///
/// In the Baumhard tree (see [`crate::mindmap::tree_builder`])
/// each section materializes as a `GfxElement::GlyphArea` child
/// of the owning node's container area, with a single
/// `GfxElement::GlyphModel` grandchild that paints the section's
/// glyphs into the section-area's buffer. The section-area is the
/// hit-test target for click routing and the carrier for
/// per-section style mutations; the section-model carries the
/// actual glyph composition.
///
/// **Defaults / inheritance.** A section without `text_runs`
/// renders at cosmic-text's defaults clamped by `node.style`:
/// the section's effective color falls through to
/// `node.style.text_color`, and the size to
/// [`DEFAULT_TEXT_RUN_SIZE_PT`]. Per-grapheme styling layers via
/// `text_runs`.
///
/// Plain data; no runtime cost beyond the `String` allocations
/// serde performs on deserialize.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MindSection {
    /// Primary text content. Styled-slice overrides live in
    /// [`Self::text_runs`]; the empty-runs / partial-runs trade-off
    /// matches the pre-refactor [`TextRun`] contract — non-empty
    /// runs render *only* the covered ranges.
    ///
    /// `#[serde(default)]` so a hand-edited or partially-converted
    /// JSON file with `{"sections": [{}]}` loads as one empty
    /// section (text = "") rather than failing the typed loader
    /// with a confusing "missing field `text`" serde error. The
    /// `convert --sections` migration synthesizes the field
    /// explicitly when lifting legacy node text, so on-disk
    /// shape is unchanged for migrated files.
    #[serde(default)]
    pub text: String,
    /// Per-grapheme styled slices — see [`TextRun`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_runs: Vec<TextRun>,
    /// Top-left of the section's AABB *relative to the owning
    /// node's `position`*, in canvas units. `(0, 0)` puts the
    /// section flush against the node's top-left.
    #[serde(default, skip_serializing_if = "is_default_position")]
    pub offset: Position,
    /// Section AABB. `None` means "fill the parent node" — the
    /// tree and scene builders compute the absolute size from the
    /// node at projection time. Authors who want a section to
    /// occupy only part of a node's AABB set this explicitly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Size>,
    /// Channel index for mutation routing inside the parent
    /// node-area. `None` means "let the tree builder substitute
    /// the section's index" — which is the migration default and
    /// what most authored sections want. `Some(0)` means "the
    /// author explicitly chose channel 0", which the builder
    /// honors even when the section's index is `> 0` (so
    /// authored 0 can intentionally collide with a sibling
    /// mind-node on channel 0 to broadcast).
    ///
    /// Sibling section channels still collide with any child
    /// mind-node sharing the same channel inside the same node —
    /// by design today. The
    /// `TargetScope::SectionsOnly` + `GfxElementField::Flag`
    /// predicate seam disambiguates at the mutation layer; this
    /// field's job is just to carry the routing intent without
    /// the `0 == default` ambiguity that the bare `usize` carried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<usize>,
    /// Per-section trigger bindings. Wired by the application's
    /// click dispatcher
    /// (`MindMapDocument::find_triggered_mutations_at`): when a
    /// click resolves to a specific section, the section's
    /// bindings fire **first**, then the whole-node bindings on
    /// `MindNode.trigger_bindings`. Per-section overrides let an
    /// author bind a different `OnClick` mutation per stratum of
    /// a multi-section node without splitting it into separate
    /// nodes.
    ///
    /// `#[serde(default)]` + `skip_serializing_if = "Vec::is_empty"`
    /// keeps the on-disk format byte-identical for sections that
    /// don't carry bindings — every existing `.mindmap.json`
    /// file remains valid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trigger_bindings: Vec<TriggerBinding>,
    /// Per-section frame style override. Drives the cyan rectangle
    /// rendered around the section while the owning node is in
    /// `InteractionMode::NodeEdit`. `None` falls through to
    /// `Canvas.default_section_frame_border` (or the focused
    /// variant when the section is currently inside the open text
    /// editor), then to a hardcoded thin / heavy floor. `Some(...)`
    /// reuses the same [`GlyphBorderConfig`] vocabulary node
    /// borders use, so per-section frames can carry any pattern,
    /// preset, font, color, or palette the border system supports.
    ///
    /// Authors set this via the `section frame …` console subverb
    /// (mirrors the `border …` keyspace).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_border: Option<GlyphBorderConfig>,
}

fn is_default_position(p: &Position) -> bool {
    p.x == 0.0 && p.y == 0.0
}

impl MindSection {
    /// Build a section that owns the given text and runs and
    /// otherwise inherits everything from the parent node — fills
    /// the node's AABB, channel 0, no offset. The right shape for
    /// the legacy single-section migration (`maptool convert
    /// --sections`) and for fresh orphan nodes that ship with one
    /// default section so users can start typing immediately.
    pub fn new_default(text: String, text_runs: Vec<TextRun>) -> Self {
        MindSection {
            text,
            text_runs,
            offset: Position::default(),
            size: None,
            channel: None,
            trigger_bindings: Vec::new(),
            frame_border: None,
        }
    }

    /// Effective size of this section for AABB containment math —
    /// the explicit `size` pin when set, otherwise `node_size`
    /// (fill-parent fallback, the migration default). Sole source
    /// of truth shared by the document-side setters
    /// (`set_section_offset` / `set_section_size`) and
    /// `maptool verify`'s containment rule, so the two cannot
    /// drift on what "fill-parent" means.
    pub fn effective_size(&self, node_size: Size) -> Size {
        self.size.unwrap_or(node_size)
    }

    /// Effective mutation channel for this section. An authored
    /// channel wins; otherwise the section's index becomes its
    /// channel, matching the tree-builder routing rule.
    pub fn effective_channel(&self, section_idx: usize) -> usize {
        self.channel.unwrap_or(section_idx)
    }
}

/// Font size in points a [`TextRun`] renders at when it does not
/// name one, and the scale a section with no runs at all is
/// measured and shaped at
/// ([`crate::mindmap::tree_builder::DEFAULT_SECTION_FONT_SCALE`]
/// is this constant by definition). One number on purpose: a run
/// that says nothing about size must render exactly like the
/// uncovered text around it, the same way a run whose `color` is
/// empty takes the color the uncovered text has.
///
/// Distinct from the *authoring* default the application writes
/// into a freshly-created run (24 pt, in the app crate's
/// `document::defaults`) — that answers "what size should new
/// text be", this answers "what size is a run that never said".
pub const DEFAULT_TEXT_RUN_SIZE_PT: f32 = 14.0;

fn default_text_run_size_pt() -> f32 {
    DEFAULT_TEXT_RUN_SIZE_PT
}
fn is_default_text_run_size_pt(v: &f32) -> bool {
    *v == DEFAULT_TEXT_RUN_SIZE_PT
}
fn is_false(v: &bool) -> bool {
    !*v
}

/// A styled slice of a section's `text`, matching miMind's text-run
/// concept: `[start, end)` grapheme indices carry one font / size /
/// color / style combination, with optional hyperlink target.
/// Multiple runs describe a single multi-style string; gaps in
/// coverage render with section-level defaults (which themselves
/// fall through to the owning node's defaults).
///
/// **Only `start` and `end` are required on disk.** Every styling
/// field defaults to the value that renders like uncovered text —
/// false flags, unpinned font, inherited color, and
/// [`DEFAULT_TEXT_RUN_SIZE_PT`] — so `{"start": 0, "end": 5,
/// "bold": true}` is a complete run. The saver omits fields
/// sitting at those defaults, per the omission policy in
/// `format/schema.md` ("a key written out at its own default value
/// may be omitted"); reloading restores the same model.
///
/// Plain data; no runtime cost beyond the string allocations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextRun {
    /// Grapheme-cluster index where this run begins (inclusive).
    pub start: usize,
    /// Grapheme-cluster index where this run ends (exclusive).
    pub end: usize,
    /// Bold weight flag. Defaults to `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    /// Italic style flag. Defaults to `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    /// Underline decoration flag. Defaults to `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    /// Font-family name; matched against `AppFont` at layout time
    /// with a fallback for unrecognized families. Empty (the
    /// default) means **no pin** — the run uses the document
    /// default face, per `format/fonts.md`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub font: String,
    /// Font size in points. Fractional sizes are first-class —
    /// the schema types this "number". Defaults to
    /// [`DEFAULT_TEXT_RUN_SIZE_PT`], the same scale run-less text
    /// renders at, so omitting it changes nothing visually.
    #[serde(
        default = "default_text_run_size_pt",
        skip_serializing_if = "is_default_text_run_size_pt"
    )]
    pub size_pt: f32,
    /// `#RRGGBB` or `var(--name)` text color. Empty (the default)
    /// declines to name one: the covered graphemes take the node's
    /// effective text color exactly as uncovered graphemes do, and
    /// keep tracking it through a retheme — see
    /// `format/text-runs.md` § "Leaving `color` empty".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    /// Optional hyperlink target URL; the renderer decorates the
    /// run's underline when set. Omitted from the JSON when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hyperlink: Option<String>,
}

/// Visual style for one node's frame / background / text. Colors are
/// raw `#RRGGBB` or `var(--name)` strings — callers pass them through
/// `util::color::resolve_var` against the canvas theme map before
/// rasterizing. Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStyle {
    /// Fill color (`#RRGGBB` or `var(--name)`).
    pub background_color: String,
    /// Border / frame color (`#RRGGBB` or `var(--name)`).
    pub frame_color: String,
    /// Default text color for the node's primary text (`#RRGGBB`
    /// or `var(--name)`).
    pub text_color: String,
    /// Background shape spelling — matched against
    /// [`crate::gfx_structs::shape::NodeShape::from_style_string`].
    /// Falls back to rectangle for any spelling with no
    /// [`NodeShape`](crate::gfx_structs::shape::NodeShape) variant
    /// behind it, quietly when the spelling is canonical
    /// ([`KNOWN_SHAPES`](crate::gfx_structs::shape::KNOWN_SHAPES))
    /// and with a `log::warn!` when it is not. Free-form either way:
    /// whatever is on disk is what gets written back.
    #[serde(default = "default_shape")]
    pub shape: String,
    /// Corner radius as a percentage of the smaller AABB dimension
    /// (0 = square corners).
    pub corner_radius_percent: f64,
    /// Frame stroke thickness in canvas units.
    pub frame_thickness: f64,
    /// When `true`, render the frame stroke at all.
    pub show_frame: bool,
    /// When `true`, render a drop shadow behind the node.
    pub show_shadow: bool,
    /// Glyph-based border configuration. Optional — if absent, the renderer
    /// applies a default border style based on the node's frame_color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<GlyphBorderConfig>,
}

/// Configures how a node's border is rendered using font glyphs.
/// All fields are optional with sensible defaults so the format stays forgiving.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlyphBorderConfig {
    /// Which glyph preset to use: "light", "heavy", "double", "rounded", or "custom"
    #[serde(default = "default_border_preset")]
    pub preset: String,
    /// Font family name for border glyphs. None = system default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Font size in points for border glyphs.
    #[serde(default = "default_border_font_size")]
    pub font_size_pt: f32,
    /// Border color override as #RRGGBB. None = inherit from frame_color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Custom glyph definitions. Only used when preset = "custom".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyphs: Option<CustomBorderGlyphs>,
    /// Padding between border and content (in pixels).
    #[serde(default = "default_border_padding")]
    pub padding: f32,
    /// Optional palette name (key in `MindMap.palettes`) whose
    /// colors cycle per glyph around the border. When absent, the
    /// resolved single color (cascade `border.color` →
    /// `style.frame_color`) paints every glyph. When present but
    /// missing from the map, the renderer logs a warning and falls
    /// back to the single-color path — interactive paths must not
    /// panic per `CODE_CONVENTIONS.md` §9.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_palette: Option<String>,
    /// Which `ColorGroup` field is cycled when `color_palette` is
    /// set: `"frame" | "background" | "text" | "title"`. Defaults
    /// to `"frame"` (the channel whose meaning matches the border
    /// today). Unknown values warn-and-fall-back to `"frame"`.
    /// Open seam for "cycle title colors" without redesigning the
    /// field later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_palette_field: Option<String>,
}

/// A [`GlyphBorderConfig`] with every field at its serde default —
/// the exact config a `"border": {}` in a `.mindmap.json`
/// deserializes to. The application's committing border setters
/// materialize this on first edit, and the border resolver's
/// unset-field floors are these same values by construction (see
/// [`crate::mindmap::border::resolve_border_style`]).
impl Default for GlyphBorderConfig {
    fn default() -> Self {
        GlyphBorderConfig {
            preset: default_border_preset(),
            font: None,
            font_size_pt: default_border_font_size(),
            color: None,
            glyphs: None,
            padding: default_border_padding(),
            color_palette: None,
            color_palette_field: None,
        }
    }
}

fn default_shape() -> String {
    "rectangle".to_string()
}
// `light` is the chosen default rather than `rounded` so corner
// glyphs (`┌┐└┘`) extend to the cell edges and join cleanly with
// the side glyphs in monospace fonts. The rounded preset's
// `╭╮╰╯` curve inward away from the cell edges, leaving a visible
// gap at every corner.
fn default_border_preset() -> String {
    "light".to_string()
}
/// Font size in points a border renders at when neither the
/// per-node config nor the canvas default names one — both the
/// serde default for [`GlyphBorderConfig::font_size_pt`] and the
/// floor of the border resolvers' size cascade
/// (`resolve_border_style` / `resolve_border_font_size_pt` in
/// `crate::mindmap::border`), which repeated the literal at three
/// sites before #47. O(1), no allocation.
pub fn default_border_font_size() -> f32 {
    14.0
}
fn default_border_padding() -> f32 {
    4.0
}

/// Custom glyphs for each part of the border.
/// Each field is a string (single char or multi-char glyph).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomBorderGlyphs {
    /// Glyph used for the horizontal top edge.
    #[serde(default = "default_h_glyph")]
    pub top: String,
    /// Glyph used for the horizontal bottom edge.
    #[serde(default = "default_h_glyph")]
    pub bottom: String,
    /// Glyph used for the vertical left edge.
    #[serde(default = "default_v_glyph")]
    pub left: String,
    /// Glyph used for the vertical right edge.
    #[serde(default = "default_v_glyph")]
    pub right: String,
    /// Glyph used for the top-left corner.
    #[serde(default = "default_tl_glyph")]
    pub top_left: String,
    /// Glyph used for the top-right corner.
    #[serde(default = "default_tr_glyph")]
    pub top_right: String,
    /// Glyph used for the bottom-left corner.
    #[serde(default = "default_bl_glyph")]
    pub bottom_left: String,
    /// Glyph used for the bottom-right corner.
    #[serde(default = "default_br_glyph")]
    pub bottom_right: String,
}

fn default_h_glyph() -> String {
    "\u{2500}".to_string()
}
fn default_v_glyph() -> String {
    "\u{2502}".to_string()
}
// Light-preset corners (┌┐└┘). Match the new `default_border_preset`
// so a `preset=custom` payload that omits a corner falls back to
// the same shape the surrounding sides connect with.
fn default_tl_glyph() -> String {
    "\u{250C}".to_string()
}
fn default_tr_glyph() -> String {
    "\u{2510}".to_string()
}
fn default_bl_glyph() -> String {
    "\u{2514}".to_string()
}
fn default_br_glyph() -> String {
    "\u{2518}".to_string()
}

/// Descriptor for how this node arranges its children — a
/// miMind-compat record carried through for round-trip fidelity.
/// Mandala does not currently drive layout from these fields;
/// custom mutations (see `format/mutations.md`) are the active
/// layout mechanism. Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeLayout {
    /// Layout-algorithm name from the miMind format — round-tripped
    /// but not currently honored by the renderer.
    #[serde(rename = "type")]
    pub layout_type: String,
    /// Growth direction hint carried through from miMind.
    pub direction: String,
    /// Inter-child spacing hint carried through from miMind.
    pub spacing: f64,
}

/// Links a node to one entry in a named [`super::Palette`] keyed by
/// depth. `level` is the index into the palette's `groups`; clamped
/// at theme-resolve time ([`super::MindMap::resolve_theme_colors`])
/// so a schema referencing a level beyond the palette's length falls
/// back to the last group rather than erroring.
/// `starts_at_root` and `connections_colored` are round-tripped
/// miMind-compat flags that the projection passes interpret when
/// resolving effective colors — see
/// [`crate::mindmap::model::theme`] for the whole cascade.
///
/// `overrides` is the one field that is not a miMind inheritance:
/// it carries this node's own exceptions to the group, and it lives
/// here rather than in [`NodeStyle`] because `style` is the tier the
/// palette shadows. See [`ColorOverrides`].
///
/// Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorSchema {
    /// Named palette to bind this node's colors to — keys into
    /// [`super::MindMap::palettes`].
    pub palette: String,
    /// Depth from the schema root, and the index into the palette's
    /// `groups` it produces. Clamped against the palette's length at
    /// resolve time so a level past the end falls back to the last
    /// group.
    ///
    /// `usize` rather than a signed type because depth cannot be
    /// negative and this value is only ever used as a slice index —
    /// the `as usize` this field used to need turned an authored
    /// `-1` into a huge index that then clamped silently to the last
    /// group. A negative `level` is now refused by the loader with
    /// serde's own message instead.
    pub level: usize,
    /// `true` when depth indexing begins at the root (level 0 is the
    /// root itself); `false` shifts the indexing so the root is
    /// transparent — it keeps its `style` colors — and its children,
    /// at `level: 1`, take `groups[0]`.
    pub starts_at_root: bool,
    /// When `true`, connections *leaving* this node inherit this
    /// node's frame tier — its `overrides.frame`, else the
    /// palette's `frame` at this node's level — instead of the
    /// edge's own `color`. Read through
    /// [`super::MindMap::edge_theme_stroke_color`], which shares
    /// that tier with the node's own border cascade so a per-node
    /// frame override cannot move one without the other.
    pub connections_colored: bool,
    /// This node's exceptions to the group above — see
    /// [`ColorOverrides`]. Absent from the JSON when empty, which is
    /// the ordinary case.
    ///
    /// An explicit `null` reads the same as a missing key.
    /// `#[serde(default)]` alone fires only on the missing key, so
    /// without the custom deserializer `"overrides": null` — the
    /// spelling every other optional object in this format accepts,
    /// and the one a generator emitting a fixed key set naturally
    /// produces — failed the whole load with
    /// `invalid type: null, expected struct ColorOverrides`.
    #[serde(
        default,
        deserialize_with = "deserialize_null_as_default",
        skip_serializing_if = "ColorOverrides::is_empty"
    )]
    pub overrides: ColorOverrides,
}

/// Per-node exceptions to a [`ColorSchema`]'s palette group — the
/// tier a direct "make *this* node green" edit lands in.
///
/// A theme is an inherited value and a per-node edit is a specific
/// one, so the edit has to win; the format already spells that rule
/// out below the node level (a [`TextRun`] naming its own `color`,
/// a [`GlyphBorderConfig::color`] on the node's border) and this is
/// the same rule at the node's own level. It cannot be expressed by
/// writing [`NodeStyle`] instead, because `style` is the tier the
/// palette *shadows*: every migrated node carries baked `style`
/// colors that are stale copies of its theme, so letting `style`
/// win would un-theme the whole corpus. Hence a channel that says
/// "overridden" by existing.
///
/// One `Option` per [`ColorGroup`] channel, `None` meaning "this
/// node has no opinion; take the group's". `title` is here for
/// completeness of the group it shadows — the interactive setters
/// cover the three channels [`NodeStyle`] also has.
///
/// Plain data; no runtime cost. Serialized only where non-empty, so
/// an untouched map round-trips byte-identically.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ColorOverrides {
    /// Replaces the group's `background` for this node alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Replaces the group's `frame` for this node alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    /// Replaces the group's `text` for this node alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Replaces the group's `title` for this node alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl ColorOverrides {
    /// `true` when no channel is overridden — the state a themed
    /// node has until somebody recolors it by hand, and the
    /// condition that keeps the key out of the serialized JSON.
    ///
    /// Cost: four `Option` discriminant reads.
    pub fn is_empty(&self) -> bool {
        self.background.is_none() && self.frame.is_none() && self.text.is_none() && self.title.is_none()
    }
}

/// One palette entry — the four colors a themed node inherits at a
/// given depth level. Referenced from [`ColorSchema::level`] via
/// [`super::Palette::groups`], resolved by
/// [`super::MindMap::resolve_theme_colors`], and also the source of
/// the per-glyph cycle a [`GlyphBorderConfig::color_palette`] border
/// draws from (which channel it cycles is
/// [`GlyphBorderConfig::color_palette_field`]). Plain data; no
/// runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorGroup {
    /// Background-fill color for a node at this level.
    pub background: String,
    /// Frame-stroke color for a node at this level.
    pub frame: String,
    /// Text color for a node at this level.
    pub text: String,
    /// First-line / title color — stands in for `text` on the first
    /// hard-newline-delimited line of the node's first
    /// [`MindSection`], the stratum miMind called the node title.
    /// Empty means "no distinct title color"; the whole section then
    /// takes `text`.
    ///
    /// Like `text`, it is a *section default*: a [`TextRun`] naming
    /// its own color keeps it, so a node whose runs all carry
    /// explicit colors shows neither channel.
    pub title: String,
}

#[cfg(test)]
mod position_clamp_tests {
    use crate::mindmap::model::validate::MAX_CANVAS_COORD;

    /// **The clamp is enforced by construction, not by remembering.**
    ///
    /// `MAX_CANVAS_COORD` is the bound with the worst history on this
    /// branch. The guard began on one writer (`set_node_aabb`), grew to
    /// three (the two layout passes), and a review round then found
    /// five more that no one had counted: the interactive drag through
    /// `apply_move_subtree` / `apply_move_single`, two animation
    /// writers, and the custom-mutation sync-back — the last reachable
    /// from a map's own trigger bindings rather than from the editor.
    /// Meanwhile the registry row for the constant claimed the writers
    /// were covered. Every one of those was found by a human reading
    /// code, which is the process this replaces.
    ///
    /// So the rule is not "clamp each writer" but "there is one
    /// writer". Anything that computes a coordinate goes through
    /// [`MindNode::set_position_clamped`] or
    /// [`MindNode::offset_position_clamped`], and this fails the build
    /// for anything that does not.
    ///
    /// What it does not cover, stated because the last three
    /// completeness claims on this branch were false: whole-struct
    /// assignment (`node.position = other`) is not a component write
    /// and is not scanned — see
    /// [`node_position_component_writes`](crate::util::source_scan::node_position_component_writes)
    /// for why. A coordinate written through a raw pointer, a
    /// `std::mem::swap`, or a macro that expands to an assignment is
    /// text this scan cannot see.
    #[test]
    fn test_every_node_position_write_goes_through_the_clamp() {
        let writes = crate::util::source_scan::node_position_component_writes();

        // `GlyphArea` also has a `position` with `x`/`y`, and the scan
        // is textual, so it sees both. The two are different rules and
        // this is the line between them: a `GlyphArea` coordinate is
        // *scene* space, deliberately unbounded, and the boundary that
        // matters is where a scene coordinate re-enters the model —
        // `sync_node_from_tree`, which is one of the sites this test
        // exists to hold. Bounding the scene as well would clamp a
        // camera-space value against a canvas-space ceiling.
        //
        // One carve-out with one reason, not a list of sites: anything
        // under `gfx_structs/` is scene space. The count is asserted
        // below so it cannot quietly grow into the whole set.
        let (scene, model): (Vec<_>, Vec<_>) = writes
            .iter()
            .partition(|(file, _, _)| file.contains("gfx_structs/"));

        let stray: Vec<_> = model
            .iter()
            .filter(|(file, _, _)| !file.ends_with("mindmap/model/node.rs"))
            .collect();
        assert!(
            stray.is_empty(),
            "these lines compute a node position and write it without the clamp:\n{}\n\n\
             Use `MindNode::set_position_clamped` (absolute) or \
             `offset_position_clamped` (delta). The loader rejects a position past \
             MAX_CANVAS_COORD, so an unclamped write authors a map that will not reopen — \
             and clamping writers one at a time is what produced five separate misses.",
            stray
                .iter()
                .map(|(f, n, line)| format!("  {f}:{n}   {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert_eq!(
            model.len(),
            2,
            "expected exactly the two writes inside `set_position_clamped`, got {model:#?}"
        );
        assert_eq!(
            scene.len(),
            12,
            "the scene-space carve-out covers the six `GlyphArea` movers in \
             `gfx_structs/area.rs` and the six on the glyph model in \
             `gfx_structs/model/glyph_model.rs` — every one a `self.position` write \
             inside the type that owns it. It now covers {} sites; check each new one is \
             really scene space and not a model position that slipped under the \
             exemption:\n{scene:#?}",
            scene.len()
        );
    }

    /// The clamp actually clamps, on both entry points, and the
    /// accumulating one clamps the **sum** rather than the delta.
    #[test]
    fn test_the_position_clamp_bounds_both_entry_points() {
        let mut node: super::MindNode =
            crate::mindmap::test_helpers::synthetic_node_full("0", None, 0.0, 0.0, 100.0, 50.0, false);

        node.set_position_clamped(1.0e30, -1.0e30);
        assert_eq!(node.position.x, MAX_CANVAS_COORD);
        assert_eq!(node.position.y, -MAX_CANVAS_COORD);

        node.set_position_clamped(f64::NAN, f64::INFINITY);
        assert_eq!(node.position.x, 0.0, "non-finite becomes zero, never NaN");
        assert_eq!(node.position.y, 0.0);

        // Walking out a step at a time is the drag/animation shape, and
        // is why the offset entry point exists at all: each delta is
        // ordinary, the running total is not.
        node.set_position_clamped(MAX_CANVAS_COORD, 0.0);
        for _ in 0..4 {
            node.offset_position_clamped(MAX_CANVAS_COORD, 0.0);
        }
        assert_eq!(
            node.position.x, MAX_CANVAS_COORD,
            "the sum is clamped, so repeated in-domain deltas cannot accumulate past the bound"
        );

        // Negative control: an ordinary move is untouched.
        node.set_position_clamped(120.0, -40.5);
        assert_eq!((node.position.x, node.position.y), (120.0, -40.5));
    }
}

#[cfg(test)]
mod section_cap_tests {
    use super::{MindNode, MAX_SECTIONS_PER_NODE};

    /// **The cap must refuse during the parse, not after it.**
    ///
    /// It lived in the loader's invariant sweep, which runs once serde
    /// has built the whole `Vec` — so a 12 MB file declaring 4 000 000
    /// sections reached 1 723 MiB resident and was *then* refused, and
    /// a file ten times larger would have died before the check ran.
    /// The constant's own doc claimed it defended against exactly that.
    ///
    /// Driven through `serde_json::from_str` rather than the loader,
    /// because that is the distinction: the loader would refuse either
    /// way, and only the typed parse failing proves nothing was built
    /// first.
    #[test]
    fn test_the_section_cap_refuses_before_building_the_vec() {
        let sections = "{},".repeat(MAX_SECTIONS_PER_NODE + 8);
        let json = format!(
            r##"{{"id":"0","parent_id":null,"position":{{"x":0,"y":0}},
                 "size":{{"width":10,"height":10}},
                 "style":{{"background_color":"#000","frame_color":"#000","text_color":"#fff",
                           "shape":"rectangle","corner_radius_percent":0,"frame_thickness":0,
                           "show_frame":false,"show_shadow":false}},
                 "layout":{{"type":"map","direction":"auto","spacing":0}},
                 "folded":false,"notes":"","sections":[{}]}}"##,
            sections.trim_end_matches(',')
        );
        let err = serde_json::from_str::<MindNode>(&json)
            .expect_err("the typed parse itself must refuse past the cap");
        assert!(
            err.to_string().contains(&MAX_SECTIONS_PER_NODE.to_string()),
            "the refusal must name the cap so the author can act on it: {err}"
        );

        // The negative control: exactly at the cap still parses, so the
        // guard cannot be passing by refusing everything.
        let ok_sections = "{},".repeat(MAX_SECTIONS_PER_NODE);
        let json = json.replace(sections.trim_end_matches(','), ok_sections.trim_end_matches(','));
        let node =
            serde_json::from_str::<MindNode>(&json).expect("a node exactly at the cap must still parse");
        assert_eq!(node.sections.len(), MAX_SECTIONS_PER_NODE);
    }
}
