// SPDX-License-Identifier: MPL-2.0

//! `MindMapDocument` — owns the data model (`MindMap`, selection,
//! undo stack, animation state, mutation registry, transient
//! previews) and hands intermediate representations to the
//! renderer. Behaviour is sharded across sibling submodules; this
//! file carries only the struct definition, construction, and the
//! scene-build entry points.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use log::{error, info};

use baumhard::mindmap::custom_mutation::CustomMutation;
use baumhard::mindmap::loader;
use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::scene_builder::{self, RenderScene};
use baumhard::mindmap::tree_builder::{self, MindMapTree};

pub mod animations;
mod custom;
pub(in crate::application) mod defaults;
mod edges;
mod hit_test;
pub mod mutations;
pub mod mutations_loader;
mod nodes;
mod topology;
mod types;
mod undo;
mod undo_action;
mod zoom_bounds;

#[cfg(test)]
pub(crate) mod tests_common;
#[cfg(test)]
mod tests_delete;
#[cfg(test)]
mod tests_edges_chain;
#[cfg(test)]
mod tests_edges_style;
#[cfg(test)]
mod tests_hit_move;
#[cfg(test)]
mod tests_mutations;
#[cfg(test)]
mod tests_nodes;
#[cfg(test)]
mod tests_reparent;
#[cfg(test)]
mod tests_resize;
#[cfg(test)]
mod tests_selection;

// Cross-platform: consumers (`scene_rebuild.rs`, `event_mouse_click.rs`,
// `run_wasm/`, `scene_host.rs`) compile on both targets. The
// plain `hit_test` (Option<String> shape) is reachable only via
// the native click handler today; the WASM click handler routes
// through `hit_test_target` (HitTarget enum). Gating the
// `hit_test` re-export to non-wasm silences the
// `#[warn(unused_imports)]` the WASM build would otherwise raise
// for an unused-on-wasm name.
#[cfg(not(target_arch = "wasm32"))]
pub use hit_test::hit_test;
pub use hit_test::{apply_tree_highlights, hit_test_target, point_in_node_aabb, HitTarget};
// Native-only: consumed by drag handlers, the click router, and
// rect-select drain — none reachable on WASM today.
#[cfg(not(target_arch = "wasm32"))]
pub use hit_test::{
    apply_drag_delta, apply_drag_delta_and_collect_patches, apply_node_resize_to_tree,
    apply_section_drag_delta_and_collect_patches, apply_section_resize_to_tree, hit_test_edge,
    hit_test_node_resize_handle, hit_test_section_resize_handle, rect_select,
};
pub use nodes::{
    BorderConfigEdits, BorderEditOutcome, BorderPreview, BorderSide, OptionEdit, SectionPayload,
};
// `BorderPreviewTarget` is consumed only by the document setters
// (and the upcoming preview verbs) — re-exported here so the
// commits adding the verb files import it from the same place
// the rest of the public document API lives. Triggers an
// unused-import warning until commit 5 lands; suppress.
#[allow(unused_imports)]
pub use nodes::BorderPreviewTarget;
pub use types::{
    AnimationInstance, EdgeLabelSel, EdgeRef, PortalLabelSel, SectionSel, SelectionState, HIGHLIGHT_COLOR,
};
// `InteractionModeOverrides` lives in baumhard (next to the
// `SceneSelectionContext` it composes into). Re-exported here so
// callers across the application crate that already
// `use crate::application::document::*` for the doc API don't have
// to reach across into baumhard's scene_builder for the value type.
pub use baumhard::mindmap::scene_builder::InteractionModeOverrides;
// Native-only: consumed by `app/click.rs`'s reparent / connect mode
// rendering. WASM doesn't dispatch `EnterReparentMode` /
// `EnterConnectMode` (NativeOnly per `wasm_compatibility`).
#[cfg(not(target_arch = "wasm32"))]
pub use types::{REPARENT_SOURCE_COLOR, REPARENT_TARGET_COLOR};
pub use undo_action::UndoAction;

/// Hard cap on `MindNode.sections.len()` enforced by `add_section`
/// and the loader. Defends against hostile mindmaps with `"sections":
/// [{},{},…10M…]` that would OOM on load. The number is generous
/// (no real authoring use case approaches 1024 sections per node)
/// and bounded enough to make exhaustion-style attacks visible.
pub const MAX_SECTIONS_PER_NODE: usize = 1024;

/// Owns the MindMap data model and provides scene-building for the Renderer.
pub struct MindMapDocument {
    pub mindmap: MindMap,
    pub file_path: Option<String>,
    pub dirty: bool,
    pub selection: SelectionState,
    pub undo_stack: Vec<UndoAction>,
    /// Registry of all available custom mutations (app + user + map +
    /// inline, keyed by id). Later layers override earlier — see
    /// [`Self::build_mutation_registry_with_app_and_user`].
    pub mutation_registry: HashMap<String, CustomMutation>,
    /// Which source layer won the registry slot for each id. Populated
    /// alongside `mutation_registry` so `mutation help <id>` can
    /// report "source: app / user / map / inline" without re-walking
    /// the layers.
    pub mutation_sources: HashMap<String, mutations_loader::MutationSource>,
    /// Per-mutation-id imperative handlers. When a handler is
    /// registered for a mutation's id, `apply_custom_mutation`
    /// delegates to it instead of the default flat-apply path — the
    /// seam size-aware / layout-generating / otherwise-Rust-computed
    /// mutations plug into. Handlers mutate the MindMap model
    /// directly; `target_scope` tells the undo path which nodes to
    /// snapshot before the handler runs.
    pub mutation_handlers: HashMap<String, mutations::DynamicMutationHandler>,
    /// Tracks active toggle mutations per node: (node_id, mutation_id).
    pub active_toggles: HashSet<(String, String)>,
    /// Currently-running animations. Each instance carries the
    /// from/to snapshot of its target node and the timing
    /// envelope; [`Self::tick_animations`] interpolates and
    /// writes the blended state back to `mindmap.nodes` until
    /// `t = 1`. Empty when no animations are active — the event
    /// loop checks [`Self::has_active_animations`] to decide
    /// whether to keep ticking. See
    /// `lib/baumhard/src/mindmap/animation.rs` for the timing /
    /// easing / lerp primitives this uses.
    pub active_animations: Vec<AnimationInstance>,
    /// Transient label edit preview. When `Some((edge_key, buffer))`,
    /// scene-building substitutes `buffer` (plus a trailing caret) for
    /// the matching edge's `ConnectionLabelElement.text` — the inline
    /// label editor's live display. Cleared on commit or cancel.
    ///
    /// Lives on the document rather than on the app layer so all
    /// `build_scene_*` callers see the override without extra
    /// plumbing. The committed `MindEdge.label` in `self.mindmap` is
    /// never touched during editing; the preview is purely a
    /// scene-level substitution.
    pub label_edit_preview: Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    /// Transient portal-text editor buffer. When `Some(...)`, the
    /// scene builder substitutes the buffer for the target
    /// endpoint's `PortalEndpointState.text` so text edits render
    /// live. Same discipline as `label_edit_preview`: the
    /// committed model in `self.mindmap` is never touched during
    /// editing; the preview is purely a scene-level substitution.
    /// Key shape is `(edge_key, endpoint_node_id, buffer)` —
    /// portal labels are per-endpoint, so the key needs both the
    /// owning edge and the endpoint side.
    pub portal_text_edit_preview: Option<(baumhard::mindmap::scene_cache::EdgeKey, String, String)>,
    /// Transient color-picker hover preview. When `Some(...)`, the
    /// scene builder substitutes the preview color for the edge
    /// under the wheel — overriding both the resolved `config.color`
    /// and any selection highlight on the previewed edge so the user
    /// sees the live HSV value on the element being edited. Fans out
    /// to the portal pass automatically for edges with
    /// `display_mode = "portal"`. Commit (`set_edge_color`) and
    /// cancel both clear the preview; neither the committed model
    /// nor the undo stack is touched during hover.
    pub color_picker_preview: Option<ColorPickerPreview>,
    /// Transient border-preview substitution. When `Some(...)`,
    /// the scene builder substitutes `edits` (folded into a clone
    /// of the committed slot) for the resolved border at the
    /// matching target — node border, section frame, or canvas
    /// default. Same discipline as the other `*_preview` fields:
    /// never serialised, never push undo, never flip `dirty`.
    /// Replaced atomically by a fresh `set_border_preview` call;
    /// cleared by `cancel_border_preview` /
    /// `commit_border_preview`; lazily ignored by the scene
    /// builder when the live selection no longer covers the
    /// preview's `selection_snapshot` (drift). Drives the
    /// `border preview …` / `section frame preview …` /
    /// `canvas border preview …` /
    /// `canvas section-frame [focused] preview …` console verbs.
    pub border_preview: Option<BorderPreview>,
}

/// Transient visual-only substitution of a color-pickerable element's
/// color. Read by `build_scene_*` and consumed by `scene_builder`'s
/// `EdgeColorPreview` and `PortalColorPreview` threaded params.
///
/// One variant handles every edge — including portal-mode edges —
/// because both routes key by the same `EdgeKey`. The scene pipeline
/// fans the preview out: the connection pass picks it up as
/// `EdgeColorPreview` when the edge renders as a line; the portal
/// pass picks it up as `PortalColorPreview` when the edge has
/// `display_mode = "portal"`.
#[derive(Debug, Clone)]
pub struct ColorPickerPreview {
    pub key: baumhard::mindmap::scene_cache::EdgeKey,
    pub color: String,
}

fn grow_node_sizes_to_fit_text(map: &mut MindMap) {
    for node in map.nodes.values_mut() {
        grow_one_node_to_fit_text(node);
    }
}

/// Per-node version of [`grow_node_sizes_to_fit_text`] — used by
/// the per-edit setters so a `font set <family>` on a single node
/// grows the box without re-walking the whole map. Same monotonic
/// "grow, never shrink" posture as the bulk pass: node sizes are
/// author intent, the loader and the per-edit setter just enforce
/// a floor.
///
/// Measures with the node's pinned font face when one is set
/// (`TextRun.font` resolves through `app_font_by_family`). Falls
/// back to cosmic-text's default when the run carries the empty
/// sentinel or names an unknown family. Without this, a node
/// pinned to a wide display face measures as if it were monospace
/// and the box undersizes by 30–60%, leaving text overflowing the
/// right edge after a `font set` or `font size=` edit.
///
/// Picks the *largest* `size_pt` across all runs rather than the
/// first — runs are usually homogeneous today (the inline editor
/// collapses to one), but a multi-size future shouldn't silently
/// fall back to the smallest measurement.
pub(super) fn grow_one_node_to_fit_text(node: &mut baumhard::mindmap::model::MindNode) {
    let (floor_w, floor_h) = compute_one_node_text_floor(node);
    if node.size.width < floor_w {
        node.size.width = floor_w;
    }
    if node.size.height < floor_h {
        node.size.height = floor_h;
    }
}

/// Pure floor-compute extracted from [`grow_one_node_to_fit_text`]
/// so the explicit-shrink path
/// [`MindMapDocument::fit_node_to_content`] can read the floor
/// without triggering the max-wins-grow side effect. Each
/// section contributes the larger of its measured text and its
/// pinned `size + offset` — pin survives when text fits;
/// overflow grows the parent so nothing visually clips.
pub(super) fn compute_one_node_text_floor(node: &baumhard::mindmap::model::MindNode) -> (f64, f64) {
    use baumhard::font::fonts::{
        acquire_font_system_write, app_font_by_family, measure_text_block_unbounded,
    };

    // §B5 lock-scope discipline: each section's measurement
    // acquires + drops the `FONT_SYSTEM` write guard
    // independently to keep parallel cargo-test workers from
    // thrashing the lock.
    let mut floor_w: f64 = 0.0;
    let mut floor_h: f64 = 0.0;
    for section in &node.sections {
        // Non-finite offsets contribute nothing — the verifier
        // flags them, and a NaN propagating into floor_w / floor_h
        // would corrupt every downstream `node.size` reader.
        if !section.offset.x.is_finite() || !section.offset.y.is_finite() {
            continue;
        }
        let scale = section
            .text_runs
            .iter()
            .map(|r| r.size_pt as f32)
            .fold(0.0_f32, f32::max);
        let scale = if scale > 0.0 { scale } else { 14.0 };
        let line_height = scale * 1.2;
        let pad_x = scale * 1.5;
        let pad_y = scale * 0.5;

        let measure_font = section
            .text_runs
            .iter()
            .max_by(|a, b| a.size_pt.cmp(&b.size_pt))
            .and_then(|r| {
                if r.font.is_empty() {
                    None
                } else {
                    app_font_by_family(&r.font)
                }
            });

        let block = {
            let mut fs = acquire_font_system_write("compute_one_node_text_floor");
            measure_text_block_unbounded(&mut fs, &section.text, scale, line_height, measure_font)
        };

        // Section dimension contribution: text needs `block + pad`
        // at minimum, but a `Some`-size section also pins a user-
        // set floor (the author wrote "this section is at least
        // this big"). Take the max so user intent survives when
        // text fits, and overflow still grows the parent so
        // nothing visually clips.
        let mut section_w = (block.width + pad_x) as f64;
        let mut section_h = (block.height + pad_y) as f64;
        if let Some(s) = section.size.as_ref() {
            if s.width.is_finite() && s.width > section_w {
                section_w = s.width;
            }
            if s.height.is_finite() && s.height > section_h {
                section_h = s.height;
            }
        }
        // Pass the offset through unmodified — the prior `.max(0)`
        // clamp silently treated leftward / upward overflow as zero,
        // hiding the actual visible-text width.
        let need_w = section_w + section.offset.x;
        let need_h = section_h + section.offset.y;
        if need_w > floor_w {
            floor_w = need_w;
        }
        if need_h > floor_h {
            floor_h = need_h;
        }
    }
    (floor_w, floor_h)
}

/// Grow every framed node's size to also accommodate its border's
/// static parts plus, when feasible, one full fill iteration on
/// each side. Mirrors [`grow_node_sizes_to_fit_text`]'s posture:
/// only grows, never shrinks — node sizes are author intent, the
/// loader and the per-edit setter just enforce a floor.
///
/// Composes monotonically with the text floor (max wins) when
/// both run on the same map.
pub(super) fn grow_node_sizes_to_fit_borders(map: &mut MindMap) {
    let canvas_default = map.canvas.default_border.clone();
    for node in map.nodes.values_mut() {
        grow_one_node_to_fit_border(node, canvas_default.as_ref());
    }
}

/// Per-node version of [`grow_node_sizes_to_fit_borders`] — used
/// by the per-edit setters so a `border preset=heavy` on a small
/// node grows the box without re-walking the whole map. The
/// canvas default is passed in so callers can hold a single
/// borrow once and re-use it.
pub(super) fn grow_one_node_to_fit_border(
    node: &mut baumhard::mindmap::model::MindNode,
    canvas_default: Option<&baumhard::mindmap::model::GlyphBorderConfig>,
) {
    use baumhard::mindmap::border::{resolve_border_style, BORDER_APPROX_CHAR_WIDTH_FRAC};
    if !node.style.show_frame {
        return;
    }
    let style = resolve_border_style(
        node.style.border.as_ref(),
        canvas_default,
        &node.style.frame_color,
    );
    let approx_char_width = style.font_size_pt * BORDER_APPROX_CHAR_WIDTH_FRAC;
    let corners = style.corner_clusters();

    // Soft target: include one full fill iteration on each side.
    // Hard floor: cover the static parts only.
    let need_top = style.side_patterns.top.minimum_with_one_fill() + corners.top_horizontal();
    let need_bottom = style.side_patterns.bottom.minimum_with_one_fill() + corners.bottom_horizontal();
    let need_left = style.side_patterns.left.minimum_with_one_fill();
    let need_right = style.side_patterns.right.minimum_with_one_fill();

    let need_horizontal_clusters = need_top.max(need_bottom);
    let need_vertical_clusters = need_left.max(need_right);

    let need_w = need_horizontal_clusters as f32 * approx_char_width;
    let need_h = need_vertical_clusters as f32 * style.font_size_pt;

    let size = node.size_vec2();
    if size.x < need_w {
        node.size.width = need_w as f64;
    }
    if size.y < need_h {
        node.size.height = need_h as f64;
    }
}

impl MindMapDocument {
    /// Wrap a `MindMap` in a fresh document shell (selection cleared,
    /// undo stack empty, mutation registry rebuilt from the map's
    /// declared mutations). Shared by `load`, `from_json_str`,
    /// `new_blank`, and the test fixture loader so the transient-
    /// state defaults stay in one place.
    ///
    /// Does **not** run [`Self::finalize`] (grow-to-fit passes) —
    /// callers must either use [`Self::load`] / [`Self::from_json_str`]
    /// (which call finalize first), or pass a map whose node sizes
    /// already accommodate its text and borders (`new_blank` —
    /// trivially; the testament fixture — by authored construction).
    pub(crate) fn from_mindmap(mindmap: MindMap, file_path: Option<String>) -> Self {
        let mut doc = MindMapDocument {
            mindmap,
            file_path,
            dirty: false,
            selection: SelectionState::None,
            undo_stack: Vec::new(),
            mutation_registry: HashMap::new(),
            mutation_sources: HashMap::new(),
            mutation_handlers: HashMap::new(),
            active_toggles: HashSet::new(),
            label_edit_preview: None,
            portal_text_edit_preview: None,
            color_picker_preview: None,
            border_preview: None,
            active_animations: Vec::new(),
        };
        doc.build_mutation_registry();
        doc
    }

    /// Load a MindMap from a file path. Native-only — WASM builds
    /// must use `from_json_str` since the browser has no filesystem.
    pub fn load(path: &str) -> Result<Self, String> {
        loader::load_from_file(Path::new(path))
            .map(|map| Self::finalize(map, Some(path.to_string())))
            .map_err(|e| {
                let msg = format!("Failed to load mindmap '{}': {}", path, e);
                error!("{}", msg);
                msg
            })
    }

    /// Construct a Document from an in-memory JSON string. `file_path`
    /// is the origin tag stored for save-back; pass the URL/path the
    /// JSON came from, or `None` for ad-hoc JSON.
    pub fn from_json_str(json: &str, file_path: Option<String>) -> Result<Self, String> {
        loader::load_from_str(json)
            .map(|map| Self::finalize(map, file_path))
            .map_err(|e| {
                error!("Failed to parse mindmap JSON: {}", e);
                e
            })
    }

    /// Grow undersized node boxes to fit their text and their
    /// border's static parts before the model is handed to the
    /// tree/scene builders. Both passes only grow, so the order
    /// composes — text-driven floor first, then border-driven —
    /// and the larger of the two wins per node.
    fn finalize(mut map: MindMap, file_path: Option<String>) -> Self {
        info!("Loaded mindmap '{}' with {} nodes", map.name, map.nodes.len());
        grow_node_sizes_to_fit_text(&mut map);
        grow_node_sizes_to_fit_borders(&mut map);
        Self::from_mindmap(map, file_path)
    }

    /// Construct an empty document, optionally bound to a target file
    /// path. Used by the `new` console command. `dirty` starts `false`
    /// — the in-memory map matches its (possibly absent) on-disk state
    /// at construction time. When `file_path` is `Some`, the caller is
    /// expected to write the blank map to disk so the binding is real;
    /// otherwise the document is "untitled" and `save` will require a
    /// path argument.
    pub fn new_blank(file_path: Option<String>) -> Self {
        let name = file_path
            .as_deref()
            .and_then(|p| {
                Path::new(p)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.trim_end_matches(".mindmap").to_string())
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "untitled".to_string());
        Self::from_mindmap(MindMap::new_blank(name), file_path)
    }

    /// Construct a doc carrying a single orphan node at the
    /// given canvas position. Test- and small-scenario fixture
    /// for "smallest interactive doc" — replaces field-by-field
    /// `MindMapDocument { ... }` literal construction at
    /// downstream test sites so the field list lives in one
    /// place.
    pub fn with_orphan(id: &str, pos: glam::Vec2) -> Self {
        let mut doc = Self::from_mindmap(MindMap::new_blank("t"), None);
        let node = super::document::defaults::default_orphan_node(id, pos);
        doc.mindmap.nodes.insert(id.to_string(), node);
        doc
    }

    /// Build a Baumhard mutation tree from the MindMap hierarchy.
    /// Each MindNode becomes a GlyphArea in the tree, preserving parent-child structure.
    pub fn build_tree(&self) -> MindMapTree {
        tree_builder::build_mindmap_tree(&self.mindmap)
    }

    /// Build a RenderScene from the current MindMap state.
    /// Used for connections and borders (flat pipeline).
    ///
    /// `camera_zoom` is forwarded through to the scene builder so
    /// connection glyphs can be sized via
    /// `GlyphConnectionConfig::effective_font_size_pt` — see
    /// `baumhard::mindmap::scene_builder::build_scene` for details.
    pub fn build_scene(&self, camera_zoom: f32) -> RenderScene {
        scene_builder::build_scene(&self.mindmap, camera_zoom)
    }

    /// The four transient scene-builder overrides every "build_scene_*"
    /// entry point on this document threads through to
    /// `baumhard::mindmap::scene_builder`: selected edge (highlight —
    /// routed to either the connection or portal pass based on the
    /// edge's `display_mode`), label-edit preview (live caret on an
    /// inline-edited edge label), and the colour-picker hover preview
    /// (fanned out to both `EdgeColorPreview` and `PortalColorPreview`
    /// so a portal-mode edge under the wheel picks it up on the
    /// marker pass). Borrowed from `&self`, so the returned tuple
    /// lives as long as `self`.
    fn assemble_scene_overrides<'a>(
        &'a self,
        resize_overrides: InteractionModeOverrides<'a>,
    ) -> (
        scene_builder::SceneSelectionContext<'a>,
        Option<scene_builder::EdgeColorPreview<'a>>,
        Option<scene_builder::PortalColorPreview<'a>>,
        Option<scene_builder::BorderPreview<'a>>,
    ) {
        let edge = self
            .selection
            .selected_edge()
            .map(|e| (e.from_id.as_str(), e.to_id.as_str(), e.edge_type.as_str()));
        // Edge-label sub-selection: when the user clicked just
        // the label (not the whole edge), only the label text
        // tints cyan. The scene builder upgrades a whole-edge
        // selection to also paint the label, so we don't need to
        // fill `edge_label` in for `Edge` selections here. The
        // `EdgeLabelSel` stores an `EdgeRef`, so we build an
        // owned `EdgeKey` per call — three small string clones,
        // negligible next to the per-frame scene build.
        let edge_label = match &self.selection {
            crate::application::document::SelectionState::EdgeLabel(s) => {
                Some(baumhard::mindmap::scene_cache::EdgeKey::from(&s.edge_ref))
            }
            _ => None,
        };
        let portal_label = self.selection.selected_portal_label_scene_ref();
        let label_edit = self.label_edit_preview.as_ref().map(|(k, s)| (k, s.as_str()));
        // Resize-handle emission + NodeEdit dimming are both driven
        // by `InteractionMode`, not by selection — the application
        // layer translates the active mode into `InteractionModeOverrides`
        // and threads it through here. Fill-parent sections emit zero
        // handles inside the scene builder regardless of the override
        // value (no own AABB to stretch).
        let selected_section = resize_overrides.section;
        let selected_node_for_resize = resize_overrides.node;
        let node_edit_for = resize_overrides.node_edit_for;
        let focused_section = resize_overrides.focused_section;
        let selection = scene_builder::SceneSelectionContext {
            edge,
            edge_label,
            portal_label,
            label_edit,
            selected_section,
            selected_node_for_resize,
            node_edit_for,
            focused_section,
        };
        let (edge_preview, portal_preview) = match &self.color_picker_preview {
            Some(ColorPickerPreview { key, color }) => (
                Some(scene_builder::EdgeColorPreview {
                    edge_key: key,
                    color: color.as_str(),
                }),
                Some(scene_builder::PortalColorPreview {
                    edge_key: key,
                    color: color.as_str(),
                }),
            ),
            None => (None, None),
        };
        // Border preview: build a borrowed scene-side view from
        // the owned `self.border_preview`. The view is borrowed
        // straight from `self`, so the returned tuple lives as
        // long as `&self`. Returns `None` when no preview is
        // active OR when the preview's target is no longer
        // covered by the live selection (defer-clear posture —
        // the actual slot empties at the next `set_*` /
        // `cancel_*` / `commit_*` call; here at scene-build
        // time, an orphan-by-drift preview just stops applying).
        let border_preview = if self.border_preview_covers_live_selection() {
            self.border_preview.as_ref().map(build_border_preview_scene_view)
        } else {
            None
        };
        (selection, edge_preview, portal_preview, border_preview)
    }

    /// Cache-aware scene build. The drag drain in `app.rs` calls this
    /// every frame with a persistent `SceneConnectionCache` so unchanged
    /// edges skip the `sample_path` geometry work entirely — Phase B of
    /// the connection-render cost fix. See
    /// `baumhard::mindmap::scene_cache` for invariants.
    ///
    /// Automatically threads the document's transient UI overrides
    /// into the scene builder:
    /// - `label_edit_preview`: live inline-label buffer + caret.
    /// - `color_picker_preview`: live color-picker hover HSV.
    pub fn build_scene_with_cache(
        &self,
        offsets: &HashMap<String, (f32, f32)>,
        cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
        camera_zoom: f32,
        resize_overrides: InteractionModeOverrides<'_>,
    ) -> RenderScene {
        let (selection, edge_preview, portal_preview, border_preview) =
            self.assemble_scene_overrides(resize_overrides);
        scene_builder::build_scene_with_cache(
            &self.mindmap,
            offsets,
            selection,
            edge_preview,
            portal_preview,
            border_preview,
            cache,
            camera_zoom,
        )
    }

    /// Build a RenderScene that also reflects the current edge selection.
    /// The selected edge (if any) gets a cyan color override baked into its
    /// ConnectionElement so the renderer paints it in the highlight color.
    ///
    /// Like `build_scene_with_cache`, this also threads the document's
    /// `label_edit_preview` and `color_picker_preview` into the scene
    /// build so live interaction previews are visible on any scene
    /// that flows through this entry point.
    pub fn build_scene_with_selection(
        &self,
        camera_zoom: f32,
        resize_overrides: InteractionModeOverrides<'_>,
    ) -> RenderScene {
        let (selection, edge_preview, portal_preview, border_preview) =
            self.assemble_scene_overrides(resize_overrides);
        scene_builder::build_scene_with_offsets_selection_and_overrides(
            &self.mindmap,
            &HashMap::new(),
            selection,
            edge_preview,
            portal_preview,
            border_preview,
            camera_zoom,
        )
    }
}

/// Build a borrowed scene-side `BorderPreview<'a>` from the owned
/// document-side `BorderPreview`. The scene-side view is `Copy +
/// 'a`; it holds `&'a str` borrows pointing at the owned
/// `BorderConfigEdits` fields, so the resulting view lives as
/// long as the document reference the caller already has.
///
/// `force_show_frame` fires when the preview's edits include any
/// preset / glyph / pattern field — preview must be visible even
/// when the committed `style.show_frame == false`, otherwise
/// `border preview preset=heavy` on a frameless node renders
/// nothing and the user thinks the verb is broken. Commit writes
/// `style.show_frame = true` through the normal setter when the
/// user wants the visibility flip persisted (today via
/// `border on`).
fn build_border_preview_scene_view<'a>(bp: &'a BorderPreview) -> scene_builder::BorderPreview<'a> {
    let target = match &bp.target {
        BorderPreviewTarget::Nodes(ids) => scene_builder::BorderPreviewTargetRef::Nodes(ids.as_slice()),
        BorderPreviewTarget::Sections(ts) => scene_builder::BorderPreviewTargetRef::Sections(ts.as_slice()),
        BorderPreviewTarget::CanvasDefault => scene_builder::BorderPreviewTargetRef::CanvasDefault,
        BorderPreviewTarget::CanvasSectionFrame => scene_builder::BorderPreviewTargetRef::CanvasSectionFrame,
        BorderPreviewTarget::CanvasSectionFrameFocused => {
            scene_builder::BorderPreviewTargetRef::CanvasSectionFrameFocused
        }
    };
    let edits = build_border_config_edits_view(&bp.edits);
    let force_show_frame = view_implies_visible(&edits);
    scene_builder::BorderPreview {
        target,
        edits,
        force_show_frame,
    }
}

/// Convert an owned `BorderConfigEdits` (from the application
/// crate) into a borrowed scene-side `BorderConfigEditsView<'a>`
/// the scene builder consumes. Per-field tri-state: `Keep` →
/// `EditView::Keep`, `Clear` → `EditView::Clear`, `Set(v)` →
/// `EditView::Set(&v)`. Pre-fix this projection collapsed both
/// `Keep` and `Clear` to a single "no edit" sentinel, dropping
/// the `Clear` axis entirely and breaking the parity contract
/// with `apply_glyph_border_edits_to_slot` (Risk #1 in the plan).
/// Test-only re-export of [`build_border_config_edits_view`].
/// Used by the parity test in `tests_nodes.rs` that exercises
/// `apply_view_to_slot` (baumhard) vs `apply_glyph_border_edits_to_slot`
/// (application) against identical edits across every per-field
/// axis. Keep `pub(crate)` — production callers go through
/// `assemble_scene_overrides`.
#[cfg(test)]
pub(crate) fn build_border_config_edits_view_for_test(
    edits: &BorderConfigEdits,
) -> scene_builder::BorderConfigEditsView<'_> {
    build_border_config_edits_view(edits)
}

/// Test-only proxy for the private `nodes::border::apply_glyph_border_edits_to_slot`
/// — keeps the module-level visibility narrow while still letting
/// the parity test in `tests_nodes.rs` exercise the helper directly.
#[cfg(test)]
pub(crate) fn nodes_border_apply_glyph_border_edits_to_slot_for_test(
    slot: &mut Option<baumhard::mindmap::model::GlyphBorderConfig>,
    edits: &BorderConfigEdits,
    outcome: &mut BorderEditOutcome,
) -> bool {
    nodes::apply_glyph_border_edits_to_slot(slot, edits, outcome)
}

fn build_border_config_edits_view(edits: &BorderConfigEdits) -> scene_builder::BorderConfigEditsView<'_> {
    use crate::application::document::OptionEdit;
    use scene_builder::EditView;
    fn opt_str(e: &OptionEdit<String>) -> EditView<&str> {
        match e {
            OptionEdit::Keep => EditView::Keep,
            OptionEdit::Clear => EditView::Clear,
            OptionEdit::Set(s) => EditView::Set(s.as_str()),
        }
    }
    fn opt_f32(e: &OptionEdit<f32>) -> EditView<f32> {
        match e {
            OptionEdit::Keep => EditView::Keep,
            OptionEdit::Clear => EditView::Clear,
            OptionEdit::Set(v) => EditView::Set(*v),
        }
    }
    fn opt_field(e: &OptionEdit<baumhard::mindmap::border::PaletteField>) -> EditView<&str> {
        match e {
            OptionEdit::Keep => EditView::Keep,
            OptionEdit::Clear => EditView::Clear,
            OptionEdit::Set(v) => EditView::Set(v.as_str()),
        }
    }
    scene_builder::BorderConfigEditsView {
        preset: opt_str(&edits.preset),
        font: opt_str(&edits.font),
        font_size_pt: opt_f32(&edits.font_size_pt),
        color: opt_str(&edits.color),
        padding: opt_f32(&edits.padding),
        color_palette: opt_str(&edits.color_palette),
        color_palette_field: opt_field(&edits.color_palette_field),
        side_top: opt_str(&edits.side_top),
        side_bottom: opt_str(&edits.side_bottom),
        side_left: opt_str(&edits.side_left),
        side_right: opt_str(&edits.side_right),
        corner_top_left: opt_str(&edits.corner_top_left),
        corner_top_right: opt_str(&edits.corner_top_right),
        corner_bottom_left: opt_str(&edits.corner_bottom_left),
        corner_bottom_right: opt_str(&edits.corner_bottom_right),
        clear: edits.clear,
    }
}

/// `true` iff `view`'s edits include at least one field that
/// implies the resolved border should be visible — any field
/// edit (`Set` or `Clear`) or the entire-slot `clear` flag.
/// Force-show then ignores a committed `style.show_frame == false`
/// for the duration of the preview so the user sees their staged
/// edits even on a frameless node.
///
/// Delegates to [`scene_builder::BorderConfigEditsView::touches_any_field`]
/// so the predicate stays in lockstep with the slot-allocation
/// gate inside `apply_view_to_slot` (the previous parallel
/// implementation drifted by one field — `clear` was excluded
/// from this side and included on the other).
fn view_implies_visible(view: &scene_builder::BorderConfigEditsView<'_>) -> bool {
    view.touches_any_field() || view.clear
}
