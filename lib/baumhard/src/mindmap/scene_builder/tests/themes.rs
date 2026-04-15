//! Theme-variable resolution across scene-builder paths: background, frame, connection, and raw fall-through on missing vars.

use super::fixtures::*;
use super::super::*;

#[test]
fn test_scene_background_resolves_theme_variable() {
    use std::collections::HashMap;
    let mut map = synthetic_map(
        vec![synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false)],
        vec![],
    );
    map.canvas.background_color = "var(--bg)".into();
    let mut vars = HashMap::new();
    vars.insert("--bg".into(), "#123456".into());
    map.canvas.theme_variables = vars;

    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.background_color, "#123456");
}

#[test]
fn test_scene_frame_color_resolves_theme_variable() {
    use std::collections::HashMap;
    let mut map = synthetic_map(
        vec![themed_node("a", "#000", "var(--frame)", "#fff")],
        vec![],
    );
    let mut vars = HashMap::new();
    vars.insert("--frame".into(), "#abcdef".into());
    map.canvas.theme_variables = vars;

    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.border_elements.len(), 1);
    // `BorderStyle::default_with_color` stores the color string as-is
    // on the style; check the resolved hex ends up there.
    let border = &scene.border_elements[0];
    assert_eq!(border.border_style.color, "#abcdef");
}

#[test]
fn test_scene_connection_color_resolves_theme_variable() {
    use std::collections::HashMap;
    let mut a = synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false);
    let mut b = synthetic_node("b", 200.0, 0.0, 40.0, 40.0, false);
    a.text = "".into(); // skip text element
    b.text = "".into();
    let mut edge = synthetic_edge("a", "b", 2, 4);
    edge.color = "var(--edge)".into();
    let mut map = synthetic_map(vec![a, b], vec![edge]);
    let mut vars = HashMap::new();
    vars.insert("--edge".into(), "#fedcba".into());
    map.canvas.theme_variables = vars;

    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.connection_elements.len(), 1);
    assert_eq!(scene.connection_elements[0].color, "#fedcba");
}

#[test]
fn test_scene_missing_variable_passes_through_raw() {
    let mut map = synthetic_map(
        vec![synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false)],
        vec![],
    );
    map.canvas.background_color = "var(--missing)".into();
    let scene = build_scene(&map, 1.0);
    // Unknown var is passed through verbatim — downstream consumers
    // decide how to handle it (hex_to_rgba_safe falls back to the
    // fallback color).
    assert_eq!(scene.background_color, "var(--missing)");
}
