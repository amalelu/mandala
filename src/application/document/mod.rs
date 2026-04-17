//! `MindMapDocument` — owns the data model (`MindMap`, selection,
//! undo stack, animation state, mutation registry, transient
//! previews) and hands intermediate representations to the
//! renderer. Pre-consolidation this file was ~5700 lines; the
//! behaviour is now sharded across sibling submodules.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use log::{error, info};

use baumhard::mindmap::custom_mutation::CustomMutation;
use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::loader;
use baumhard::mindmap::scene_builder::{self, RenderScene};
use baumhard::mindmap::tree_builder::{self, MindMapTree};

mod animations;
mod custom;
mod defaults;
mod edges;
mod hit_test;
mod nodes;
mod topology;
mod types;
mod undo;
mod undo_action;

#[cfg(test)]
mod tests_common;
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

pub use hit_test::{
    apply_drag_delta, apply_drag_delta_and_collect_patches,
    apply_tree_highlights, hit_test, hit_test_edge,
    point_in_node_aabb, rect_select,
};
pub use types::{
    AnimationInstance, EdgeRef, PortalLabelSel, SelectionState, HIGHLIGHT_COLOR,
    REPARENT_SOURCE_COLOR, REPARENT_TARGET_COLOR,
};
pub use undo_action::UndoAction;

/// Owns the MindMap data model and provides scene-building for the Renderer.
pub struct MindMapDocument {
    pub mindmap: MindMap,
    pub file_path: Option<String>,
    pub dirty: bool,
    pub selection: SelectionState,
    pub undo_stack: Vec<UndoAction>,
    /// Registry of all available custom mutations (global + map + inline, keyed by id).
    pub mutation_registry: HashMap<String, CustomMutation>,
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
pub enum ColorPickerPreview {
    Edge {
        key: baumhard::mindmap::scene_cache::EdgeKey,
        color: String,
    },
}

fn grow_node_sizes_to_fit_text(map: &mut MindMap) {
    use cosmic_text::{Attrs, Buffer, Metrics, Shaping};

    let mut font_system = baumhard::font::fonts::FONT_SYSTEM
        .write()
        .expect("font system lock poisoned");

    for node in map.nodes.values_mut() {
        let scale = node
            .text_runs
            .first()
            .map(|r| r.size_pt as f32)
            .unwrap_or(14.0);
        let line_height = scale * 1.2;
        let pad_x = scale * 1.5;
        let pad_y = scale * 0.5;

        let mut buffer = Buffer::new(&mut font_system, Metrics::new(scale, line_height));
        // Unbounded layout so we measure the natural single-line width
        // of each logical line (cosmic-text still breaks on embedded
        // `\n`), which is the right floor for "how big does the box
        // need to be".
        buffer.set_size(&mut font_system, None, None);
        buffer.set_text(
            &mut font_system,
            &node.text,
            &Attrs::new(),
            Shaping::Advanced,
            None,
        );

        let measured_w = buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0_f32, f32::max);
        let measured_h = buffer.layout_runs().count() as f32 * line_height;

        let need_w = (measured_w + pad_x) as f64;
        let need_h = (measured_h + pad_y) as f64;
        if node.size.width < need_w {
            node.size.width = need_w;
        }
        if node.size.height < need_h {
            node.size.height = need_h;
        }
    }
}

impl MindMapDocument {
    /// Wrap a `MindMap` in a fresh document shell (selection cleared,
    /// undo stack empty, mutation registry rebuilt from the map's
    /// declared mutations). Shared by `load` and `new_blank` so the
    /// transient-state defaults stay in one place.
    fn from_mindmap(mindmap: MindMap, file_path: Option<String>) -> Self {
        let mut doc = MindMapDocument {
            mindmap,
            file_path,
            dirty: false,
            selection: SelectionState::None,
            undo_stack: Vec::new(),
            mutation_registry: HashMap::new(),
            active_toggles: HashSet::new(),
            label_edit_preview: None,
            color_picker_preview: None,
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

    /// Grow undersized node boxes to fit their text before the model
    /// is handed to the tree/scene builders — see
    /// `grow_node_sizes_to_fit_text` for the invariants.
    fn finalize(mut map: MindMap, file_path: Option<String>) -> Self {
        info!("Loaded mindmap '{}' with {} nodes", map.name, map.nodes.len());
        grow_node_sizes_to_fit_text(&mut map);
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

    /// Build a RenderScene with position offsets applied to specific nodes.
    /// Used during drag to update connections and borders in real-time.
    pub fn build_scene_with_offsets(
        &self,
        offsets: &HashMap<String, (f32, f32)>,
        camera_zoom: f32,
    ) -> RenderScene {
        scene_builder::build_scene_with_offsets(&self.mindmap, offsets, camera_zoom)
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
    fn assemble_scene_overrides(
        &self,
    ) -> (
        Option<(&str, &str, &str)>,
        Option<scene_builder::SelectedPortalLabel<'_>>,
        Option<(&baumhard::mindmap::scene_cache::EdgeKey, &str)>,
        Option<scene_builder::EdgeColorPreview<'_>>,
        Option<scene_builder::PortalColorPreview<'_>>,
    ) {
        let sel = self
            .selection
            .selected_edge()
            .map(|e| (e.from_id.as_str(), e.to_id.as_str(), e.edge_type.as_str()));
        let selected_portal_label = self.selection.selected_portal_label_scene_ref();
        let label_edit = self
            .label_edit_preview
            .as_ref()
            .map(|(k, s)| (k, s.as_str()));
        let (edge_preview, portal_preview) = match &self.color_picker_preview {
            Some(ColorPickerPreview::Edge { key, color }) => (
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
        (sel, selected_portal_label, label_edit, edge_preview, portal_preview)
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
    ) -> RenderScene {
        let (sel, selected_portal_label, label_edit, edge_preview, portal_preview) =
            self.assemble_scene_overrides();
        scene_builder::build_scene_with_cache(
            &self.mindmap,
            offsets,
            sel,
            selected_portal_label,
            label_edit,
            edge_preview,
            portal_preview,
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
    pub fn build_scene_with_selection(&self, camera_zoom: f32) -> RenderScene {
        let (sel, selected_portal_label, label_edit, edge_preview, portal_preview) =
            self.assemble_scene_overrides();
        scene_builder::build_scene_with_offsets_selection_and_overrides(
            &self.mindmap,
            &HashMap::new(),
            sel,
            selected_portal_label,
            label_edit,
            edge_preview,
            portal_preview,
            camera_zoom,
        )
    }
}
