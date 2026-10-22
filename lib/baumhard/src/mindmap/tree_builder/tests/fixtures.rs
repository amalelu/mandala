// SPDX-License-Identifier: MPL-2.0

//! Shared fixtures for the tree_builder tests. Bodies live in the
//! shared `mindmap::test_helpers` module — this file thins over
//! that canonical surface with the per-call-site signatures the
//! tree_builder tests grew used to.
//!
//! Two node shapes coexist because the two halves of the builder
//! want different defaults: [`synthetic_node`] pins 80×40 with the
//! frame on and takes a parent (hierarchy tests), while
//! [`sized_node`] takes explicit dimensions + `show_frame`
//! (geometry and clipping tests). Both delegate to
//! [`crate::mindmap::test_helpers::synthetic_node_full`].

use crate::mindmap::model::{MindMap, MindNode};

pub(super) use crate::mindmap::test_helpers::{
    synthetic_edge, synthetic_map, synthetic_portal_edge, testament_map_path as test_map_path,
};

/// Hierarchy shape: 80×40 box, frame on, parent-aware.
pub(super) fn synthetic_node(id: &str, parent: Option<&str>, x: f64, y: f64) -> MindNode {
    crate::mindmap::test_helpers::synthetic_node_full(id, parent, x, y, 80.0, 40.0, true)
}

/// Geometry shape: explicit size + `show_frame`, no parent.
pub(super) fn sized_node(id: &str, x: f64, y: f64, w: f64, h: f64, show_frame: bool) -> MindNode {
    crate::mindmap::test_helpers::synthetic_node_full(id, None, x, y, w, h, show_frame)
}

/// A [`sized_node`] with the three theme-driven color fields
/// overridden — the fixture the theme-variable resolution tests
/// build on.
pub(super) fn themed_node(id: &str, bg: &str, frame: &str, text: &str) -> MindNode {
    let mut n = sized_node(id, 0.0, 0.0, 40.0, 40.0, true);
    n.style.background_color = bg.to_string();
    n.style.frame_color = frame.to_string();
    n.style.text_color = text.to_string();
    n
}

/// Two frameless 40×40 nodes 400px apart joined by one
/// right→left edge — the minimal fixture for connection sampling,
/// clipping, label, and handle tests.
pub(super) fn two_node_edge_map() -> MindMap {
    synthetic_map(
        vec![
            sized_node("a", 0.0, 0.0, 40.0, 40.0, false),
            sized_node("b", 400.0, 0.0, 40.0, 40.0, false),
        ],
        vec![synthetic_edge("a", "b", "right", "left")],
    )
}

/// Builds an N-node linear spine: `n0 -> n1 -> n2 -> ... -> n{N-1}`.
/// Useful for depth-stress tests and O(N²) regression guards.
pub(super) fn mk_chain_map(n: usize) -> MindMap {
    assert!(n >= 1);
    let mut nodes = Vec::with_capacity(n);
    nodes.push(synthetic_node("c0", None, 0.0, 0.0));
    for i in 1..n {
        let parent = format!("c{}", i - 1);
        let id = format!("c{}", i);
        nodes.push(synthetic_node(&id, Some(&parent), 0.0, i as f64 * 50.0));
    }
    synthetic_map(nodes, vec![])
}

/// Builds a star: one root and `n - 1` sibling children.
pub(super) fn mk_star_map(n: usize) -> MindMap {
    assert!(n >= 1);
    let mut nodes = Vec::with_capacity(n);
    nodes.push(synthetic_node("root", None, 0.0, 0.0));
    for i in 1..n {
        let id = format!("s{}", i);
        nodes.push(synthetic_node(&id, Some("root"), (i as f64) * 100.0, 100.0));
    }
    synthetic_map(nodes, vec![])
}

pub(super) fn glyph_area_of(
    tree: &crate::gfx_structs::tree::Tree<
        crate::gfx_structs::element::GfxElement,
        crate::gfx_structs::mutator::GfxMutator,
    >,
    node_id: indextree::NodeId,
) -> &crate::gfx_structs::area::GlyphArea {
    tree.arena.get(node_id).unwrap().get().glyph_area().unwrap()
}

/// Shorthand for the `Tree` every builder in this module emits.
pub(super) type GfxTree = crate::gfx_structs::tree::Tree<
    crate::gfx_structs::element::GfxElement,
    crate::gfx_structs::mutator::GfxMutator,
>;

/// A `GlyphArea`'s clickable rectangle as `(top_left, extent)` —
/// the same pair the BVH tests against.
pub(super) fn area_rect(area: &crate::gfx_structs::area::GlyphArea) -> (glam::Vec2, glam::Vec2) {
    (
        glam::Vec2::new(area.position.x.0, area.position.y.0),
        glam::Vec2::new(area.render_bounds.x.0, area.render_bounds.y.0),
    )
}

/// Center point of a `GlyphArea`'s rectangle — the canonical
/// "click right on this thing" probe.
pub(super) fn area_center(area: &crate::gfx_structs::area::GlyphArea) -> glam::Vec2 {
    let (pos, extent) = area_rect(area);
    crate::util::geometry::aabb_center(pos, extent)
}

/// The `(icon, text)` `GlyphArea` pair under endpoint `endpoint_idx`
/// of the first portal pair in a built portal tree. Cloned so the
/// caller can go on to borrow the tree mutably for a hit-test.
pub(super) fn endpoint_leaf_areas(
    tree: &GfxTree,
    endpoint_idx: usize,
) -> (
    crate::gfx_structs::area::GlyphArea,
    crate::gfx_structs::area::GlyphArea,
) {
    let pair = tree.root.children(&tree.arena).next().expect("one portal pair");
    let endpoint = pair
        .children(&tree.arena)
        .nth(endpoint_idx)
        .expect("endpoint void");
    let leaves: Vec<_> = endpoint.children(&tree.arena).collect();
    (
        glyph_area_of(tree, leaves[0]).clone(),
        glyph_area_of(tree, leaves[1]).clone(),
    )
}

/// Run the production hit path over a built portal tree: BVH
/// descent, then identity resolution through the tree's own index.
pub(super) fn portal_hit_at(
    portal: &mut super::super::PortalTree,
    point: glam::Vec2,
) -> Option<super::super::PortalHit> {
    let node_id = portal.tree.descendant_at(point)?;
    portal.hit_index.resolve(&portal.tree, node_id)
}

/// Every leaf of a built portal tree that can actually be clicked
/// (strictly positive extent), named by sub-part in tree order.
/// Zero-extent reserved text slots are absent by construction.
pub(super) fn hittable_parts(portal: &super::super::PortalTree) -> Vec<super::super::PortalPart> {
    let mut out = Vec::new();
    for pair in portal.tree.root.children(&portal.tree.arena) {
        for endpoint in pair.children(&portal.tree.arena) {
            for leaf in endpoint.children(&portal.tree.arena) {
                let area = glyph_area_of(&portal.tree, leaf);
                let (_, extent) = area_rect(area);
                if extent.x <= 0.0 || extent.y <= 0.0 {
                    continue;
                }
                if let Some(hit) = portal.hit_index.resolve(&portal.tree, leaf) {
                    out.push(hit.part);
                }
            }
        }
    }
    out
}

/// The per-role outputs of one rebuild that cross-role assertions
/// read, in one value.
///
/// **Test scaffolding, not a pipeline stage**: no such aggregate
/// exists in the crate or the app. `CanvasFrame` in the application
/// crate runs these same passes and hands each result straight to
/// its canvas role. Tests that assert against two or three roles
/// from one fixture map want them together, so
/// [`project_with_cache`] runs the passes against one shared
/// fold-hidden set and one shared clip-AABB pass — exactly as the
/// app does — and collects the results here.
///
/// Roles whose tests only ever look at one output
/// (section frames, the two resize-handle families) call their own
/// pass directly and are deliberately absent.
pub(super) struct ProjectedRoles {
    pub connection_elements: Vec<super::super::ConnectionElement>,
    pub edge_handles: Vec<super::super::EdgeHandleElement>,
    pub connection_label_elements: Vec<super::super::ConnectionLabelElement>,
    pub border_nodes: Vec<super::super::BorderNodeData>,
    /// Portal pair data flattened to one entry per endpoint — the
    /// shape marker-level assertions want.
    pub portal_markers: Vec<PortalMarker>,
    /// Clip boxes (`top_left`, `size`) the connection pass filtered
    /// samples against.
    pub node_aabbs: Vec<(glam::Vec2, glam::Vec2)>,
    /// How many distinct `ConnectionPath`s the two connection passes
    /// built between them — [`super::super::EdgePathCache::built`] on
    /// the memo they shared.
    pub paths_built: usize,
}

/// Run every per-role pass against `map`, in the order
/// `CanvasFrame::update_all` does.
///
/// Argument order mirrors the per-pass override plumbing:
/// selection first, then the two color previews, then the border
/// preview, then the persistent connection cache and the camera
/// zoom the sample spacing depends on.
#[allow(clippy::too_many_arguments)]
pub(super) fn project_with_cache(
    map: &MindMap,
    offsets: &std::collections::HashMap<String, (f32, f32)>,
    selection: super::super::SceneSelectionContext<'_>,
    edge_color: Option<super::super::EdgeColorPreview<'_>>,
    portal_color: Option<super::super::PortalColorPreview<'_>>,
    border: Option<super::super::BorderPreview<'_>>,
    cache: &mut crate::mindmap::scene_cache::SceneConnectionCache,
    camera_zoom: f32,
) -> ProjectedRoles {
    use super::super::{
        border_node_data, build_connection_elements, build_label_elements, node_clip_aabbs, portal_pair_data,
        BorderChromeOverrides,
    };
    use crate::mindmap::scene_cache::EdgeKey;

    cache.ensure_zoom(camera_zoom);
    let hidden = map.fold_hidden_set();
    let portal_pairs = portal_pair_data(
        map,
        offsets,
        selection.edge,
        selection.portal_label,
        portal_color,
        None,
        camera_zoom,
        &hidden,
    );
    let node_aabbs = node_clip_aabbs(map, offsets, border, &hidden);
    // One memo across both passes, exactly as `CanvasFrame` owns one
    // across its `update_*` methods — a fixture that gave each pass
    // its own would report the sharing as absent whether or not it
    // worked.
    let mut paths = super::super::EdgePathCache::new(map, offsets);
    let (connection_elements, edge_handles) = build_connection_elements(
        map,
        offsets,
        &node_aabbs,
        selection.edge,
        edge_color,
        cache,
        &mut paths,
        camera_zoom,
        &hidden,
    );
    let highlight_key: Option<EdgeKey> = selection
        .edge_label
        .clone()
        .or_else(|| selection.edge.map(|(f, t, ty)| EdgeKey::new(f, t, ty)));
    let connection_label_elements = build_label_elements(
        map,
        selection.label_edit,
        edge_color,
        highlight_key.as_ref(),
        &mut paths,
        camera_zoom,
        &hidden,
    );
    let paths_built = paths.built();
    ProjectedRoles {
        connection_elements,
        edge_handles,
        connection_label_elements,
        paths_built,
        border_nodes: border_node_data(
            map,
            offsets,
            BorderChromeOverrides {
                preview: border,
                node_edit_for: selection.node_edit_for,
            },
            &hidden,
        ),
        portal_markers: portal_markers(&portal_pairs),
        node_aabbs,
    }
}

/// [`project_with_cache`] with a throwaway cache — the shape most
/// single-build assertions want.
#[allow(clippy::too_many_arguments)]
pub(super) fn project_with_overrides(
    map: &MindMap,
    offsets: &std::collections::HashMap<String, (f32, f32)>,
    selection: super::super::SceneSelectionContext<'_>,
    edge_color: Option<super::super::EdgeColorPreview<'_>>,
    portal_color: Option<super::super::PortalColorPreview<'_>>,
    border: Option<super::super::BorderPreview<'_>>,
    camera_zoom: f32,
) -> ProjectedRoles {
    let mut cache = crate::mindmap::scene_cache::SceneConnectionCache::new();
    project_with_cache(
        map,
        offsets,
        selection,
        edge_color,
        portal_color,
        border,
        &mut cache,
        camera_zoom,
    )
}

/// [`project_with_overrides`] with no offsets and no overrides.
pub(super) fn project(map: &MindMap, camera_zoom: f32) -> ProjectedRoles {
    project_with_overrides(
        map,
        &std::collections::HashMap::new(),
        super::super::SceneSelectionContext::default(),
        None,
        None,
        None,
        camera_zoom,
    )
}

/// [`project`] with drag offsets applied.
pub(super) fn project_with_offsets(
    map: &MindMap,
    offsets: &std::collections::HashMap<String, (f32, f32)>,
    camera_zoom: f32,
) -> ProjectedRoles {
    project_with_overrides(
        map,
        offsets,
        super::super::SceneSelectionContext::default(),
        None,
        None,
        None,
        camera_zoom,
    )
}

/// One portal endpoint's rendered marker, flattened out of the
/// nested [`ProjectedRoles::portal_pairs`] shape.
///
/// The portal pass emits `pairs[i].endpoints[0..2]` (index 0 =
/// `from_id`, index 1 = `to_id`), each carrying full `GlyphArea`s.
/// Marker-level assertions want the resolved scalars instead, so
/// [`portal_markers`] reads them back *off the projected areas* —
/// which makes those assertions cover the stamping step too, not
/// just the resolver's return value.
pub(super) struct PortalMarker {
    pub edge_key: crate::mindmap::scene_cache::EdgeKey,
    pub endpoint_node_id: String,
    pub glyph: String,
    pub position: (f32, f32),
    pub bounds: (f32, f32),
    /// Lowercase `#rrggbb` / `#rrggbbaa`, re-formatted from the
    /// icon's single color region.
    pub color: String,
    pub font_size_pt: f32,
    pub zoom_visibility: crate::gfx_structs::zoom_visibility::ZoomVisibility,
}

/// Flatten portal pair data into per-endpoint markers, in
/// `(pair, endpoint)` order.
pub(super) fn portal_markers(pairs: &[super::super::PortalPairData]) -> Vec<PortalMarker> {
    pairs
        .iter()
        .flat_map(|pair| {
            pair.endpoints.iter().map(|ep| {
                let icon = &ep.icon;
                let rgba = icon
                    .regions
                    .all_regions()
                    .first()
                    .and_then(|r| r.color)
                    .unwrap_or([0.0, 0.0, 0.0, 1.0]);
                PortalMarker {
                    edge_key: pair.identity.clone(),
                    endpoint_node_id: ep.endpoint_node_id.clone(),
                    glyph: icon.text.clone(),
                    position: (icon.position.x.0, icon.position.y.0),
                    bounds: (icon.render_bounds.x.0, icon.render_bounds.y.0),
                    color: crate::util::color::rgba_to_hex(rgba),
                    font_size_pt: icon.scale.0,
                    zoom_visibility: icon.zoom_visibility,
                }
            })
        })
        .collect()
}
