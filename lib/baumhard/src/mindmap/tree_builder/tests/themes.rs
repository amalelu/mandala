// SPDX-License-Identifier: MPL-2.0

//! Theme-variable resolution across the projection passes: canvas
//! background, node frame, connection body, and the raw
//! fall-through on a missing variable.

use super::fixtures::*;

/// The renderer reads `Canvas.background_color` directly and
/// resolves it at clear-color time, so the contract under test is
/// the resolver itself against a fixture-authored variant table.
#[test]
fn test_canvas_background_resolves_theme_variable() {
    use std::collections::HashMap;
    let mut map = synthetic_map(vec![sized_node("a", 0.0, 0.0, 40.0, 40.0, false)], vec![]);
    map.canvas.background_color = "var(--bg)".into();
    let mut vars = HashMap::new();
    vars.insert("--bg".into(), "#123456".into());
    map.canvas.theme_variables = vars;

    assert_eq!(
        crate::util::color::resolve_var(&map.canvas.background_color, &map.canvas.theme_variables),
        "#123456"
    );
}

#[test]
fn test_border_frame_color_resolves_theme_variable() {
    use std::collections::HashMap;
    let mut map = synthetic_map(vec![themed_node("a", "#000", "var(--frame)", "#fff")], vec![]);
    let mut vars = HashMap::new();
    vars.insert("--frame".into(), "#abcdef".into());
    map.canvas.theme_variables = vars;

    let projected = project(&map, 1.0);
    assert_eq!(projected.border_nodes.len(), 1);
    // The frame cascade resolves the variable before it reaches
    // `BorderStyle`, which stores the resulting hex verbatim.
    assert_eq!(projected.border_nodes[0].border_style.color, "#abcdef");
}

#[test]
fn test_connection_color_resolves_theme_variable() {
    use std::collections::HashMap;
    let mut a = sized_node("a", 0.0, 0.0, 40.0, 40.0, false);
    let mut b = sized_node("b", 200.0, 0.0, 40.0, 40.0, false);
    a.sections[0].text = "".into();
    b.sections[0].text = "".into();
    let mut edge = synthetic_edge("a", "b", "right", "left");
    edge.color = "var(--edge)".into();
    let mut map = synthetic_map(vec![a, b], vec![edge]);
    let mut vars = HashMap::new();
    vars.insert("--edge".into(), "#fedcba".into());
    map.canvas.theme_variables = vars;

    let projected = project(&map, 1.0);
    assert_eq!(projected.connection_elements.len(), 1);
    assert_eq!(projected.connection_elements[0].color, "#fedcba");
}

#[test]
fn test_missing_variable_passes_through_raw() {
    let mut map = synthetic_map(vec![sized_node("a", 0.0, 0.0, 40.0, 40.0, false)], vec![]);
    map.canvas.background_color = "var(--missing)".into();
    // Unknown var is passed through verbatim — downstream consumers
    // decide how to handle it (hex_to_rgba_safe falls back to the
    // fallback color).
    assert_eq!(
        crate::util::color::resolve_var(&map.canvas.background_color, &map.canvas.theme_variables),
        "var(--missing)"
    );
}
