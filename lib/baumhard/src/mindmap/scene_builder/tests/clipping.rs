//! Connection glyph clipping behaviour: inside-node clip, frame-area clip, cap survival for unframed endpoints, cap clip for framed endpoints.

use super::fixtures::*;
use super::super::*;
use crate::mindmap::model::GlyphConnectionConfig;
use std::collections::HashMap;
use glam::Vec2;

#[test]
fn test_scene_clips_connection_glyphs_inside_node() {
    // A on the left, B on the right, blocker C directly on the path
    // between them. The A→B connection should skip body glyphs that
    // fall inside C. All three nodes are unframed so only the raw
    // AABB clipping is exercised here.
    let map = synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
            synthetic_node("c", 180.0, 0.0, 60.0, 40.0, false),
        ],
        vec![synthetic_edge("a", "b", 2, 4)], // right edge of A → left edge of B
    );

    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.connection_elements.len(), 1);
    let conn = &scene.connection_elements[0];

    // No body glyph position should fall strictly inside C's AABB.
    for &(x, y) in &conn.glyph_positions {
        let inside_c = x > 180.5 && x < 239.5 && y > 0.5 && y < 39.5;
        assert!(!inside_c,
            "glyph at ({}, {}) should have been clipped by blocker C",
            x, y);
    }
    assert!(!conn.glyph_positions.is_empty(),
        "some glyphs should remain outside the blocker");
}

#[test]
fn test_scene_clips_connection_glyphs_in_frame_area() {
    // Same A→B→blocker layout but this time C has a visible frame.
    // The border at default 14pt font extends ~8.4 px horizontally and
    // ~14 px vertically past C's AABB, so body glyphs in the expanded
    // region should also be clipped.
    let border_font = 14.0_f32;
    let border_char_w = border_font * 0.6;

    let map = synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
            synthetic_node("c", 180.0, 0.0, 60.0, 40.0, true),
        ],
        vec![synthetic_edge("a", "b", 2, 4)],
    );

    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.connection_elements.len(), 1);
    let conn = &scene.connection_elements[0];

    // The clip AABB for framed C is expanded by (border_char_w,
    // border_font) on every side. No body glyph should fall inside
    // the expanded region.
    let min_x = 180.0 - border_char_w + 0.5;
    let max_x = 240.0 + border_char_w - 0.5;
    let min_y = 0.0 - border_font + 0.5;
    let max_y = 40.0 + border_font - 0.5;
    for &(x, y) in &conn.glyph_positions {
        let inside_expanded_c =
            x > min_x && x < max_x && y > min_y && y < max_y;
        assert!(!inside_expanded_c,
            "glyph at ({}, {}) should have been clipped by framed C's expanded AABB",
            x, y);
    }
    // Body glyphs should still render in the space between A, C's
    // expanded clip box, and B.
    assert!(!conn.glyph_positions.is_empty(),
        "connection between A and B should still have visible body glyphs outside C's frame");
}

#[test]
fn test_scene_caps_survive_for_unframed_endpoints() {
    // A→B connection with a cap_start glyph configured. Because A and
    // B are unframed, the anchor point sits exactly on A's edge and
    // the cap should render there.
    use crate::mindmap::model::GlyphConnectionConfig;
    let mut edge = synthetic_edge("a", "b", 2, 4);
    edge.glyph_connection = Some(GlyphConnectionConfig {
        body: "·".into(),
        cap_start: Some("►".into()),
        cap_end: Some("◄".into()),
        font: None,
        font_size_pt: 12.0,
        color: None,
        spacing: 0.0,
        ..GlyphConnectionConfig::default()
    });
    let map = synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
        ],
        vec![edge],
    );
    let scene = build_scene(&map, 1.0);
    let conn = &scene.connection_elements[0];
    assert!(conn.cap_start.is_some(),
        "cap_start should survive for unframed source");
    assert!(conn.cap_end.is_some(),
        "cap_end should survive for unframed target");
}

#[test]
fn test_scene_caps_clipped_for_framed_endpoints() {
    // A→B connection where the target B has a visible frame. The
    // cap_end sits on B's node edge, which is STRICTLY inside B's
    // frame-expanded clip AABB, so it should be dropped — otherwise
    // the cap would render in the visible border area.
    use crate::mindmap::model::GlyphConnectionConfig;
    let mut edge = synthetic_edge("a", "b", 2, 4);
    edge.glyph_connection = Some(GlyphConnectionConfig {
        body: "·".into(),
        cap_start: Some("►".into()),
        cap_end: Some("◄".into()),
        font: None,
        font_size_pt: 12.0,
        color: None,
        spacing: 0.0,
        ..GlyphConnectionConfig::default()
    });
    let map = synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, true), // framed!
        ],
        vec![edge],
    );
    let scene = build_scene(&map, 1.0);
    let conn = &scene.connection_elements[0];
    // Source is unframed — cap_start still shows at A's right edge.
    assert!(conn.cap_start.is_some(),
        "cap_start should survive for unframed source");
    // Target is framed — cap_end falls inside the expanded clip AABB.
    assert!(conn.cap_end.is_none(),
        "cap_end should be clipped when target has a visible frame");
}

// --- Phase B cache tests --------------------------------------------
