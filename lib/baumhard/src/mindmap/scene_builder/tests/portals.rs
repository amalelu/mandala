//! Portal element emission tests. Portals are now edges with
//! `display_mode = "portal"` — these tests verify two-marker
//! emission, missing/folded endpoint filtering, theme-variable
//! resolution, selection highlight, top-right anchor, drag offset
//! follow. Line-mode edges stay in the connection pipeline and are
//! covered by the connection tests.

use super::fixtures::*;
use super::super::*;
use crate::mindmap::scene_cache::{EdgeKey, SceneConnectionCache};
use glam::Vec2;
use std::collections::HashMap;

#[test]
fn portal_emits_two_elements_per_edge() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 500.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.portal_elements.len(), 2);
    let ids: Vec<&str> = scene
        .portal_elements
        .iter()
        .map(|e| e.endpoint_node_id.as_str())
        .collect();
    assert!(ids.contains(&"a"));
    assert!(ids.contains(&"b"));
    // Both markers share the same edge identity.
    assert_eq!(
        scene.portal_elements[0].edge_key,
        scene.portal_elements[1].edge_key
    );
}

#[test]
fn portal_skipped_when_endpoint_missing_from_map() {
    let nodes = vec![synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false)];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges
        .push(synthetic_portal_edge("a", "ghost", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    assert!(
        scene.portal_elements.is_empty(),
        "missing endpoint should silently drop the portal-mode edge"
    );
}

#[test]
fn portal_skipped_when_either_endpoint_hidden_by_fold() {
    // A parent holding a folded child — the child is hidden by
    // fold from its ancestor. A portal edge pointing into a folded
    // subtree has no visible anchor, so it must be skipped.
    let mut root = synthetic_node("root", 0.0, 0.0, 60.0, 40.0, false);
    root.folded = true;
    let mut child = synthetic_node("child", 200.0, 0.0, 60.0, 40.0, false);
    child.parent_id = Some("root".to_string());
    let other = synthetic_node("other", 500.0, 0.0, 60.0, 40.0, false);
    let mut map = synthetic_map(vec![root, child, other], vec![]);
    map.edges
        .push(synthetic_portal_edge("child", "other", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    assert!(
        scene.portal_elements.is_empty(),
        "portal edge should be dropped when one endpoint is hidden by fold"
    );
}

#[test]
fn portal_color_resolves_through_theme_variable() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.canvas
        .theme_variables
        .insert("--accent".to_string(), "#ff00aa".to_string());
    map.edges
        .push(synthetic_portal_edge("a", "b", "var(--accent)"));
    let scene = build_scene(&map, 1.0);
    assert_eq!(
        scene.portal_elements[0].color, "#ff00aa",
        "var(--accent) must resolve through theme_variables"
    );
    assert_eq!(scene.portal_elements[1].color, "#ff00aa");
}

#[test]
fn selected_portal_edge_rendered_with_highlight_color() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext {
            edge: Some(("a", "b", "cross_link")),
            ..Default::default()
        },
        None,
        None,
        &mut cache,
        1.0,
    );
    // Both emitted markers flip to the cyan highlight color.
    assert_eq!(scene.portal_elements[0].color, "#00E5FF");
    assert_eq!(scene.portal_elements[1].color, "#00E5FF");
}

#[test]
fn portal_marker_points_at_partner_endpoint() {
    // Post-directional-default: the marker sits on the owning
    // node's border at the point facing its partner endpoint,
    // not floated above a fixed corner. Partner "b" is to the
    // south-east of "a", so a's marker anchors on the
    // right / bottom side.
    let nodes = vec![
        synthetic_node("a", 100.0, 200.0, 80.0, 40.0, false),
        synthetic_node("b", 500.0, 500.0, 80.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    let marker_a = scene
        .portal_elements
        .iter()
        .find(|e| e.endpoint_node_id == "a")
        .expect("marker for endpoint a");
    // Center of marker AABB should sit outside the node's right
    // or bottom side (the two sides facing partner b). Check that
    // the marker lives outside the node AABB in at least one axis.
    let marker_cx = marker_a.position.0 + marker_a.bounds.0 * 0.5;
    let marker_cy = marker_a.position.1 + marker_a.bounds.1 * 0.5;
    let right_edge = 100.0 + 80.0;
    let bottom_edge = 200.0 + 40.0;
    assert!(
        marker_cx > right_edge || marker_cy > bottom_edge,
        "marker ({marker_cx}, {marker_cy}) should be right or below node AABB"
    );
}

#[test]
fn portal_marker_follows_drag_offsets() {
    // When the owning node is offset during a drag, the marker
    // moves with it. The relationship between marker shift and
    // node shift is not strict equality because the directional
    // default also uses the partner's position (which is
    // unchanged here), but the marker stays anchored to the
    // moved node's border — so its delta has the same sign and
    // similar magnitude in each axis.
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));

    let baseline = build_scene(&map, 1.0);
    let baseline_a = baseline
        .portal_elements
        .iter()
        .find(|e| e.endpoint_node_id == "a")
        .expect("marker for endpoint a in baseline");

    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (100.0f32, 50.0f32));
    let dragged = build_scene_with_offsets(&map, &offsets, 1.0);
    let dragged_a = dragged
        .portal_elements
        .iter()
        .find(|e| e.endpoint_node_id == "a")
        .expect("marker for endpoint a in dragged scene");

    let dx = dragged_a.position.0 - baseline_a.position.0;
    let dy = dragged_a.position.1 - baseline_a.position.1;
    // Partner stays to the right → marker stays on a's right
    // edge, x shifts by roughly the node's x offset. y shift is
    // somewhere between 0 and the node's y offset depending on
    // whether the anchor slid vertically to face the new partner
    // direction.
    assert!(dx > 50.0, "marker x should follow node rightward, got {dx}");
    assert!(dy >= -10.0 && dy <= 60.0, "marker y shift {dy} within node move range");
}

#[test]
fn portal_mode_edge_skipped_by_connection_pipeline() {
    // A portal-mode edge must not produce a `ConnectionElement`
    // (no line, no body glyphs). The connection pass is the one
    // place where this would leak into rendering.
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    assert!(
        scene.connection_elements.is_empty(),
        "portal-mode edges must not produce connection elements"
    );
    assert_eq!(scene.portal_elements.len(), 2);
}

#[test]
fn portal_color_preview_wins_over_selection() {
    // Picker hover feedback on a selected portal edge must show the
    // preview color on both markers, not the cyan selection highlight.
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let key = EdgeKey::new("a", "b", "cross_link");
    let preview = PortalColorPreview { edge_key: &key, color: "#112233" };
    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext {
            edge: Some(("a", "b", "cross_link")),
            ..Default::default()
        },
        None,
        Some(preview),
        &mut cache,
        1.0,
    );
    assert_eq!(scene.portal_elements[0].color, "#112233");
    assert_eq!(scene.portal_elements[1].color, "#112233");
}

// ---- portal text-style resolver (text_color / text_font_size_pt /
// text_min_font_size_pt / text_max_font_size_pt) ----

#[test]
fn portal_text_style_inherits_icon_color_when_override_absent() {
    // Per-endpoint `text_color` is absent → resolver returns the
    // already-resolved icon color. Preserves the pre-refactor
    // behaviour for maps that don't opt into the new text channel.
    use super::super::portal::resolve_portal_endpoint_text_style;
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let edge = &map.edges[0];
    let icon_color = "#aa88cc";
    let text_style = resolve_portal_endpoint_text_style(
        edge,
        None,
        &map.canvas,
        None,
        icon_color,
        1.0,
    );
    assert_eq!(text_style.color, icon_color);
}

#[test]
fn portal_text_color_override_wins_over_icon_cascade() {
    use crate::mindmap::model::PortalEndpointState;
    use super::super::portal::resolve_portal_endpoint_text_style;
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let endpoint = PortalEndpointState {
        text_color: Some("#00ff00".to_string()),
        ..Default::default()
    };
    let text_style = resolve_portal_endpoint_text_style(
        &map.edges[0],
        Some(&endpoint),
        &map.canvas,
        None,
        "#aa88cc",
        1.0,
    );
    assert_eq!(text_style.color, "#00ff00");
}

#[test]
fn portal_text_color_transient_override_wins_over_endpoint_override() {
    // Selection cyan / preview hex must beat the per-endpoint
    // `text_color` so wheel drag stays visible while a label is
    // selected.
    use crate::mindmap::model::PortalEndpointState;
    use super::super::portal::resolve_portal_endpoint_text_style;
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let endpoint = PortalEndpointState {
        text_color: Some("#00ff00".to_string()),
        ..Default::default()
    };
    let text_style = resolve_portal_endpoint_text_style(
        &map.edges[0],
        Some(&endpoint),
        &map.canvas,
        Some("#ffffff"),
        "#aa88cc",
        1.0,
    );
    assert_eq!(text_style.color, "#ffffff");
}

#[test]
fn portal_text_font_size_override_wins_over_icon_base() {
    // Per-endpoint `text_font_size_pt` detaches the text from the
    // icon so a coloured badge can host a smaller annotation beside
    // it without shrinking the badge itself.
    use crate::mindmap::model::PortalEndpointState;
    use super::super::portal::resolve_portal_endpoint_text_style;
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let endpoint = PortalEndpointState {
        text_font_size_pt: Some(24.0),
        text_min_font_size_pt: Some(12.0),
        text_max_font_size_pt: Some(96.0),
        ..Default::default()
    };
    // At zoom 1.0, target screen = 24 pt, well inside [12, 96] → 24.
    let text_style = resolve_portal_endpoint_text_style(
        &map.edges[0],
        Some(&endpoint),
        &map.canvas,
        None,
        "#aa88cc",
        1.0,
    );
    assert!(
        (text_style.font_size_pt - 24.0).abs() < 1.0e-4,
        "expected 24, got {}",
        text_style.font_size_pt
    );
    // At zoom 0.25, target screen = 6 pt → pinned at min 12 → canvas
    // size = 12 / 0.25 = 48.
    let text_style_zoomed_out = resolve_portal_endpoint_text_style(
        &map.edges[0],
        Some(&endpoint),
        &map.canvas,
        None,
        "#aa88cc",
        0.25,
    );
    assert!(
        (text_style_zoomed_out.font_size_pt - 48.0).abs() < 1.0e-4,
        "expected 48 after zoom clamp, got {}",
        text_style_zoomed_out.font_size_pt
    );
}

#[test]
fn portal_text_font_size_inherits_icon_default_when_absent() {
    // No `text_font_size_pt` override → text size equals the icon
    // size (which inherits from `glyph_connection` or the portal
    // default).
    use super::super::portal::{
        resolve_portal_endpoint_style, resolve_portal_endpoint_text_style,
    };
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let icon = resolve_portal_endpoint_style(
        &map.edges[0],
        None,
        &map.canvas,
        None,
        1.0,
    );
    let text = resolve_portal_endpoint_text_style(
        &map.edges[0],
        None,
        &map.canvas,
        None,
        &icon.color,
        1.0,
    );
    assert!((icon.font_size_pt - text.font_size_pt).abs() < 1.0e-4);
}

#[test]
fn portal_text_aabb_never_overlaps_icon_aabb() {
    // Regression for the diagonal-normal AABB overlap bug: the
    // icon and text AABBs are both world-axis-aligned, so pushing
    // the text `center` outward by `icon_half + padding + text_half`
    // along a ~45° normal still let the text AABB slip back into
    // the icon's bounds for long text. The `layout_portal_text`
    // support-function fix guarantees non-overlap for every
    // `border_t` the user can reach. Exercise several positions
    // around the border — including the cardinal-corner
    // transitions in `border_outward_normal` — and every text
    // length from 1 char to a realistic long label.
    use super::super::portal::{layout_portal_label, layout_portal_text, PortalLabelLayout};
    use crate::mindmap::model::PortalEndpointState;

    let owner_pos = Vec2::new(100.0, 100.0);
    let owner_size = Vec2::new(200.0, 80.0);
    let partner_center = Vec2::new(1000.0, 500.0);
    let icon_font = 50.0;
    let text_font = 14.0;

    // Walk the full border parameter range in 64 steps — covers
    // every side plus the cardinal-corner transitions where the
    // normal direction jumps.
    for i in 0..64 {
        let t = (i as f32 / 64.0) * 4.0;
        let state = PortalEndpointState {
            border_t: Some(t),
            ..Default::default()
        };
        let icon = layout_portal_label(
            owner_pos,
            owner_size,
            partner_center,
            Some(&state),
            icon_font,
        );
        for text in [
            "x",
            "hello",
            "a much longer annotation label that could reach back",
        ] {
            let layout = layout_portal_text(
                icon,
                owner_pos,
                owner_size,
                partner_center,
                Some(&state),
                icon_font,
                text_font,
                text,
            );
            // Icon AABB.
            let icon_min = icon.top_left;
            let icon_max = icon.top_left + icon.bounds;
            let text_min = layout.top_left;
            let text_max = layout.top_left + layout.bounds;
            // Two AABBs are disjoint iff max of one is less than
            // min of the other on some axis.
            let disjoint_x = text_max.x <= icon_min.x || text_min.x >= icon_max.x;
            let disjoint_y = text_max.y <= icon_min.y || text_min.y >= icon_max.y;
            assert!(
                disjoint_x || disjoint_y,
                "text AABB overlaps icon AABB at border_t={t} text={text:?}: \
                 icon=[{:?}..{:?}] text=[{:?}..{:?}]",
                icon_min, icon_max, text_min, text_max,
            );
        }
    }

    // Ensure the synthetic_* helpers are still reachable from
    // this test module under the new test — touching to prevent
    // a future import-pruning pass from silently deleting.
    let _ = synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false);
    let _ = synthetic_portal_edge("a", "b", "#aa88cc");
}

/// Zoom-visibility cascade (scene-builder path): when both
/// endpoint bounds are `None`, the emitted `PortalElement`
/// inherits the edge's window verbatim.
#[test]
fn portal_zoom_visibility_inherits_edge_window_when_endpoint_absent() {
    use crate::gfx_structs::zoom_visibility::ZoomVisibility;

    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    let mut edge = synthetic_portal_edge("a", "b", "#aa88cc");
    edge.min_zoom_to_render = Some(0.5);
    edge.max_zoom_to_render = Some(2.0);
    map.edges.push(edge);
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.portal_elements.len(), 2);
    for pe in &scene.portal_elements {
        assert_eq!(
            pe.zoom_visibility,
            ZoomVisibility { min: Some(0.5), max: Some(2.0) },
        );
    }
}

/// Zoom-visibility cascade (scene-builder path): the endpoint's
/// pair **replaces** the edge's pair whenever either bound is
/// `Some`, mirroring the portal text-clamp cascade. This test
/// sets only `min` on one endpoint and confirms the other
/// endpoint still inherits the edge window — the cascade is
/// per-endpoint, not shared between the two markers of an edge.
#[test]
fn portal_zoom_visibility_endpoint_override_replaces_edge_window() {
    use crate::gfx_structs::zoom_visibility::ZoomVisibility;
    use crate::mindmap::model::PortalEndpointState;

    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    let mut edge = synthetic_portal_edge("a", "b", "#aa88cc");
    edge.min_zoom_to_render = Some(0.5);
    edge.max_zoom_to_render = Some(2.0);
    edge.portal_from = Some(PortalEndpointState {
        min_zoom_to_render: Some(1.5),
        max_zoom_to_render: None,
        ..Default::default()
    });
    map.edges.push(edge);
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.portal_elements.len(), 2);
    let from = scene
        .portal_elements
        .iter()
        .find(|pe| pe.endpoint_node_id == "a")
        .expect("portal_from endpoint present");
    let to = scene
        .portal_elements
        .iter()
        .find(|pe| pe.endpoint_node_id == "b")
        .expect("portal_to endpoint present");
    assert_eq!(
        from.zoom_visibility,
        ZoomVisibility { min: Some(1.5), max: None },
        "endpoint override must fully replace the edge window, not intersect"
    );
    assert_eq!(
        to.zoom_visibility,
        ZoomVisibility { min: Some(0.5), max: Some(2.0) },
        "the untouched endpoint must still inherit the edge window",
    );
}

/// Defaults: an edge with no authored window emits two portal
/// elements whose `zoom_visibility` is unbounded.
#[test]
fn portal_zoom_visibility_defaults_to_unbounded() {
    use crate::gfx_structs::zoom_visibility::ZoomVisibility;

    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    for pe in &scene.portal_elements {
        assert_eq!(pe.zoom_visibility, ZoomVisibility::unbounded());
    }
}

/// A `PortalEndpointState` that is `Some(state)` but whose
/// zoom bounds are both `None` must still inherit the edge's
/// window — presence of the state alone does not trigger the
/// replace branch. Pins the `cascade_replace((None, None)) →
/// inherit` semantics at the portal-endpoint level; a
/// regression that treated any `Some(state)` as a replace
/// would silently un-gate portals with per-endpoint color or
/// border_t overrides.
#[test]
fn portal_zoom_visibility_inherits_when_endpoint_has_no_zoom_bounds() {
    use crate::gfx_structs::zoom_visibility::ZoomVisibility;
    use crate::mindmap::model::PortalEndpointState;

    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    let mut edge = synthetic_portal_edge("a", "b", "#aa88cc");
    edge.min_zoom_to_render = Some(0.5);
    edge.max_zoom_to_render = Some(2.0);
    // `Some(state)` but no zoom bounds — state carries a
    // color override, not a window.
    edge.portal_from = Some(PortalEndpointState {
        color: Some("#ff0000".to_string()),
        min_zoom_to_render: None,
        max_zoom_to_render: None,
        ..Default::default()
    });
    map.edges.push(edge);
    let scene = build_scene(&map, 1.0);
    let from = scene
        .portal_elements
        .iter()
        .find(|pe| pe.endpoint_node_id == "a")
        .expect("portal_from endpoint present");
    assert_eq!(
        from.zoom_visibility,
        ZoomVisibility { min: Some(0.5), max: Some(2.0) },
        "Some(endpoint) with no zoom bounds must inherit the edge window"
    );
}

/// Portal-text AABB width is driven by grapheme count, not Unicode-
/// scalar count. A family-ZWJ emoji between two letters is three
/// graphemes but nine scalars; without §B3 compliance the AABB
/// would be sized at nine slots. Guards against a revert to
/// `.chars().count()` on the portal-text layout path.
#[test]
fn test_portal_text_bounds_use_grapheme_count_not_scalar_count() {
    use super::super::portal::{layout_portal_label, layout_portal_text};
    use crate::mindmap::model::PortalEndpointState;

    let owner_pos = Vec2::new(100.0, 100.0);
    let owner_size = Vec2::new(200.0, 80.0);
    let partner_center = Vec2::new(1000.0, 500.0);
    let icon_font = 50.0;
    let text_font = 14.0;
    let state = PortalEndpointState {
        border_t: Some(0.0),
        ..Default::default()
    };
    let icon = layout_portal_label(
        owner_pos,
        owner_size,
        partner_center,
        Some(&state),
        icon_font,
    );
    let plain = layout_portal_text(
        icon,
        owner_pos,
        owner_size,
        partner_center,
        Some(&state),
        icon_font,
        text_font,
        "XYZ",
    );
    // "A👨\u{200D}👩\u{200D}👧B" — 9 scalars, 3 graphemes.
    let zwj = layout_portal_text(
        icon,
        owner_pos,
        owner_size,
        partner_center,
        Some(&state),
        icon_font,
        text_font,
        "A\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}B",
    );
    assert!(
        (plain.bounds.x - zwj.bounds.x).abs() < f32::EPSILON,
        "grapheme-sized bounds expected; got plain={} zwj={}",
        plain.bounds.x, zwj.bounds.x,
    );
}

/// Portal-text layout must size one-grapheme labels the same across
/// every grapheme-formation mechanism: regional-indicator pairs,
/// skin-tone modifiers, and combining marks. Each of the three uses
/// a distinct Unicode mechanism that the ZWJ family emoji above does
/// not exercise, and §T1 asks Unicode fundamentals to be tested
/// broadly.
#[test]
fn test_portal_text_bounds_regional_indicator_pair_is_single_grapheme() {
    assert_portal_text_one_grapheme_equivalent("\u{1F1EF}\u{1F1F5}"); // 🇯🇵
}

#[test]
fn test_portal_text_bounds_skin_tone_modifier_is_single_grapheme() {
    assert_portal_text_one_grapheme_equivalent("\u{1F44B}\u{1F3FE}"); // 👋🏾
}

#[test]
fn test_portal_text_bounds_combining_mark_is_single_grapheme() {
    assert_portal_text_one_grapheme_equivalent("n\u{0303}");
}

/// An empty portal-text string must still produce a non-zero AABB:
/// `layout_portal_text`'s `.max(1)` floor holds the slot to at
/// least one grapheme's width so an empty buffer doesn't collapse
/// to a zero-sized layout the scene builder would then try to
/// divide by. Pins that clamp.
#[test]
fn test_portal_text_bounds_empty_string_uses_one_grapheme_floor() {
    use super::super::portal::{layout_portal_label, layout_portal_text};
    use crate::mindmap::model::PortalEndpointState;

    let owner_pos = Vec2::new(100.0, 100.0);
    let owner_size = Vec2::new(200.0, 80.0);
    let partner_center = Vec2::new(1000.0, 500.0);
    let icon_font = 50.0;
    let text_font = 14.0;
    let state = PortalEndpointState {
        border_t: Some(0.0),
        ..Default::default()
    };
    let icon = layout_portal_label(
        owner_pos,
        owner_size,
        partner_center,
        Some(&state),
        icon_font,
    );
    let empty = layout_portal_text(
        icon,
        owner_pos,
        owner_size,
        partner_center,
        Some(&state),
        icon_font,
        text_font,
        "",
    );
    let one_char = layout_portal_text(
        icon,
        owner_pos,
        owner_size,
        partner_center,
        Some(&state),
        icon_font,
        text_font,
        "X",
    );
    assert!(
        (empty.bounds.x - one_char.bounds.x).abs() < f32::EPSILON,
        "empty portal-text should match 1-grapheme width via the .max(1) floor; got empty={} one_char={}",
        empty.bounds.x, one_char.bounds.x,
    );
    assert!(
        empty.bounds.x > 0.0,
        "empty portal-text bounds must be strictly positive",
    );
}

/// Helper — build a portal-text layout for a one-grapheme string
/// and assert its bounds match a single-ASCII-character baseline.
fn assert_portal_text_one_grapheme_equivalent(text: &str) {
    use super::super::portal::{layout_portal_label, layout_portal_text};
    use crate::mindmap::model::PortalEndpointState;

    let owner_pos = Vec2::new(100.0, 100.0);
    let owner_size = Vec2::new(200.0, 80.0);
    let partner_center = Vec2::new(1000.0, 500.0);
    let icon_font = 50.0;
    let text_font = 14.0;
    let state = PortalEndpointState {
        border_t: Some(0.0),
        ..Default::default()
    };
    let icon = layout_portal_label(
        owner_pos,
        owner_size,
        partner_center,
        Some(&state),
        icon_font,
    );
    let baseline = layout_portal_text(
        icon,
        owner_pos,
        owner_size,
        partner_center,
        Some(&state),
        icon_font,
        text_font,
        "X",
    );
    let actual = layout_portal_text(
        icon,
        owner_pos,
        owner_size,
        partner_center,
        Some(&state),
        icon_font,
        text_font,
        text,
    );
    assert!(
        (baseline.bounds.x - actual.bounds.x).abs() < f32::EPSILON,
        "portal text {:?} should size as one grapheme; got {} vs. baseline {}",
        text, actual.bounds.x, baseline.bounds.x,
    );
}
