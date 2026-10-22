// SPDX-License-Identifier: MPL-2.0

//! The palette cascade as the *projection passes* see it.
//!
//! `model::theme`'s own tests pin the resolver; these pin the thing
//! the resolver exists for — that a themed node's fill, frame, text
//! and outgoing edge stroke actually come out of `map.palettes`, and
//! that editing a palette moves them. Every assertion here fails if
//! the cascade is unwired, because every node in the fixtures carries
//! `style` colors deliberately different from its palette group.
//!
//! The on-disk half runs against `maps/palette_cascade.mindmap.json`,
//! which exists for exactly this: the older `theme_demo` fixture
//! carries no palettes at all, and `testament`'s baked `style` values
//! have drifted from its schemas, so neither can tell a live cascade
//! from a dead one.

use super::super::*;
use super::fixtures::*;
use crate::mindmap::model::{ColorGroup, ColorOverrides, ColorSchema, MindMap, Palette};

/// Distinct sentinel per channel per group — a reader that grabs
/// the wrong one cannot accidentally agree with the right one.
fn probe_palette() -> Palette {
    Palette {
        groups: vec![
            ColorGroup {
                background: "#a10000".into(),
                frame: "#a20000".into(),
                text: "#a30000".into(),
                title: "#a40000".into(),
            },
            ColorGroup {
                background: "#b10000".into(),
                frame: "#b20000".into(),
                text: "#b30000".into(),
                title: "#b40000".into(),
            },
        ],
    }
}

fn schema(level: usize, starts_at_root: bool, connections_colored: bool) -> ColorSchema {
    ColorSchema {
        palette: "probe".into(),
        level,
        starts_at_root,
        connections_colored,
        overrides: ColorOverrides::default(),
    }
}

/// Two framed nodes joined `a -> b`, palette installed, nothing
/// themed yet. `synthetic_node` pins `#000` fill and `#fff` frame /
/// text, so no palette sentinel can collide with the fallback.
fn probe_map() -> MindMap {
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", Some("a"), 400.0, 0.0),
        ],
        vec![synthetic_edge("a", "b", "right", "left")],
    );
    map.palettes.insert("probe".into(), probe_palette());
    map
}

/// Path to `maps/palette_cascade.mindmap.json`, the fixture whose
/// whole job is to have a palette that differs from every node's
/// `style`.
fn cascade_map() -> MindMap {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // lib/baumhard -> lib
    path.pop(); // lib -> root
    path.push("maps/palette_cascade.mindmap.json");
    crate::mindmap::loader::load_from_file(&path).expect("the palette-cascade fixture must load")
}

/// The single color region of a section-area with no text runs.
fn section_color(tree: &MindMapTree, mind_id: &str, section_idx: usize) -> Vec<String> {
    let sid = tree
        .section_arena_id(mind_id, section_idx)
        .expect("section must project");
    glyph_area_of(&tree.tree, sid)
        .regions
        .all_regions()
        .into_iter()
        .map(|r| crate::util::color::rgba_to_hex(r.color.expect("regions carry a color")))
        .collect()
}

fn frame_color_of(map: &MindMap, mind_id: &str) -> String {
    let projected = project(map, 1.0);
    projected
        .border_nodes
        .iter()
        .find(|b| b.node_id == mind_id)
        .expect("a framed node must reach the border pass")
        .border_style
        .color
        .clone()
}

// ---- node fill -------------------------------------------------

#[test]
fn test_node_fill_comes_from_the_palette_group() {
    let mut map = probe_map();
    map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
    let tree = build_mindmap_tree(&map);
    let area = glyph_area_of(&tree.tree, tree.arena_id_for("a").unwrap());
    assert_eq!(
        area.background_color,
        Some([0xa1, 0, 0, 255]),
        "the themed node must fill from groups[0].background, not style.background_color"
    );
}

#[test]
fn test_node_fill_falls_back_to_style_without_a_schema() {
    let map = probe_map();
    let tree = build_mindmap_tree(&map);
    let area = glyph_area_of(&tree.tree, tree.arena_id_for("a").unwrap());
    assert_eq!(area.background_color, Some([0, 0, 0, 255]));
}

// ---- node frame ------------------------------------------------

#[test]
fn test_node_frame_comes_from_the_palette_group() {
    let mut map = probe_map();
    map.nodes.get_mut("b").unwrap().color_schema = Some(schema(1, true, false));
    assert_eq!(frame_color_of(&map, "b"), "#b20000");
}

#[test]
fn test_node_frame_falls_back_to_style_without_a_schema() {
    let map = probe_map();
    assert_eq!(frame_color_of(&map, "b"), "#fff");
}

/// A per-node `border.color` is an explicit choice and still beats
/// the theme it sits on top of.
#[test]
fn test_an_explicit_border_color_outranks_the_palette_frame() {
    use crate::mindmap::model::GlyphBorderConfig;
    let mut map = probe_map();
    {
        let node = map.nodes.get_mut("b").unwrap();
        node.color_schema = Some(schema(1, true, false));
        node.style.border = Some(GlyphBorderConfig {
            preset: "light".into(),
            font: None,
            font_size_pt: 14.0,
            color: Some("#00ff00".into()),
            glyphs: None,
            padding: 4.0,
            color_palette: None,
            color_palette_field: None,
        });
    }
    assert_eq!(frame_color_of(&map, "b"), "#00ff00");
}

// ---- node text -------------------------------------------------

#[test]
fn test_runless_section_text_comes_from_the_palette_group() {
    let mut map = probe_map();
    map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
    map.nodes.get_mut("a").unwrap().sections[0].text = "one line".into();
    map.nodes.get_mut("a").unwrap().sections[0].text_runs.clear();
    let tree = build_mindmap_tree(&map);
    assert_eq!(section_color(&tree, "a", 0), vec!["#a30000".to_string()]);
}

#[test]
fn test_runless_section_text_falls_back_to_style_without_a_schema() {
    let mut map = probe_map();
    map.nodes.get_mut("a").unwrap().sections[0].text = "one line".into();
    map.nodes.get_mut("a").unwrap().sections[0].text_runs.clear();
    map.nodes.get_mut("a").unwrap().style.text_color = "#0000ff".into();
    let tree = build_mindmap_tree(&map);
    assert_eq!(section_color(&tree, "a", 0), vec!["#0000ff".to_string()]);
}

/// Section 0's first hard-newline-delimited line takes the group's
/// `title` channel; everything after it takes `text`.
#[test]
fn test_first_line_of_the_title_section_takes_the_title_channel() {
    let mut map = probe_map();
    {
        let node = map.nodes.get_mut("a").unwrap();
        node.color_schema = Some(schema(0, true, false));
        node.sections[0].text = "title\nbody".into();
        node.sections[0].text_runs.clear();
    }
    let tree = build_mindmap_tree(&map);
    assert_eq!(
        section_color(&tree, "a", 0),
        vec!["#a40000".to_string(), "#a30000".to_string()]
    );
}

/// A run that names its own color is an authored per-slice
/// override; a retheme must not repaint it.
#[test]
fn test_an_authored_run_color_survives_the_theme() {
    use crate::mindmap::model::TextRun;
    let mut map = probe_map();
    {
        let node = map.nodes.get_mut("a").unwrap();
        node.color_schema = Some(schema(0, true, false));
        node.sections[0].text = "abc".into();
        node.sections[0].text_runs = vec![TextRun {
            start: 0,
            end: 3,
            bold: false,
            italic: false,
            underline: false,
            font: String::new(),
            size_pt: 24.0,
            color: "#123456".into(),
            hyperlink: None,
        }];
    }
    let tree = build_mindmap_tree(&map);
    assert_eq!(section_color(&tree, "a", 0), vec!["#123456".to_string()]);
}

/// …but a run that names *no* color inherits the node default, so
/// the theme reaches it.
#[test]
fn test_a_colorless_run_inherits_the_palette_text() {
    use crate::mindmap::model::TextRun;
    let mut map = probe_map();
    {
        let node = map.nodes.get_mut("a").unwrap();
        node.color_schema = Some(schema(0, true, false));
        node.sections[0].text = "abc".into();
        node.sections[0].text_runs = vec![TextRun {
            start: 0,
            end: 3,
            bold: false,
            italic: false,
            underline: false,
            font: String::new(),
            size_pt: 24.0,
            color: String::new(),
            hyperlink: None,
        }];
    }
    let tree = build_mindmap_tree(&map);
    assert_eq!(section_color(&tree, "a", 0), vec!["#a30000".to_string()]);
}

// ---- edges -----------------------------------------------------

#[test]
fn test_connections_colored_paints_the_edge_from_the_source_group() {
    let mut map = probe_map();
    map.edges[0].color = "#0000ff".into();
    map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, true));
    map.nodes.get_mut("b").unwrap().color_schema = Some(schema(1, true, true));
    let projected = project(&map, 1.0);
    assert_eq!(projected.connection_elements.len(), 1);
    assert_eq!(
        projected.connection_elements[0].color, "#a20000",
        "the edge takes its from_id node's group frame"
    );
}

#[test]
fn test_connections_colored_off_leaves_the_edges_own_color() {
    let mut map = probe_map();
    map.edges[0].color = "#0000ff".into();
    map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
    let projected = project(&map, 1.0);
    assert_eq!(projected.connection_elements[0].color, "#0000ff");
}

// ---- the headline claim ----------------------------------------

/// `format/palettes.md`: "Editing a palette is a single-point
/// change; every node using it updates on the next render." This is
/// the test that claim rests on — one write to `map.palettes`, and
/// every projected role moves.
#[test]
fn test_editing_the_palette_changes_the_projected_output() {
    let mut map = probe_map();
    map.edges[0].color = "#0000ff".into();
    for id in ["a", "b"] {
        let node = map.nodes.get_mut(id).unwrap();
        node.color_schema = Some(schema(0, true, true));
        node.sections[0].text = "text".into();
        node.sections[0].text_runs.clear();
    }

    let before = build_mindmap_tree(&map);
    assert_eq!(
        glyph_area_of(&before.tree, before.arena_id_for("a").unwrap()).background_color,
        Some([0xa1, 0, 0, 255])
    );
    assert_eq!(frame_color_of(&map, "a"), "#a20000");
    // One line, no newline, so the title channel does not apply and
    // the whole section takes `text`.
    assert_eq!(section_color(&before, "a", 0), vec!["#a30000".to_string()]);
    assert_eq!(project(&map, 1.0).connection_elements[0].color, "#a20000");

    // One edit, at the map level. No node is touched.
    map.palettes.get_mut("probe").unwrap().groups[0] = ColorGroup {
        background: "#0a0b0c".into(),
        frame: "#101112".into(),
        text: "#202122".into(),
        title: "#303132".into(),
    };

    let tree = build_mindmap_tree(&map);
    assert_eq!(
        glyph_area_of(&tree.tree, tree.arena_id_for("a").unwrap()).background_color,
        Some([0x0a, 0x0b, 0x0c, 255]),
        "fill must follow the edited palette"
    );
    assert_eq!(frame_color_of(&map, "a"), "#101112", "frame must follow it too");
    assert_eq!(
        section_color(&tree, "a", 0),
        vec!["#202122".to_string()],
        "and so must the text"
    );
    assert_eq!(
        project(&map, 1.0).connection_elements[0].color,
        "#101112",
        "and the connection, via connections_colored"
    );
}

// ---- the on-disk fixture ---------------------------------------

/// Subtree 0 of `maps/palette_cascade.mindmap.json`: `starts_at_root`
/// is true, so the root itself is themed at `groups[0]`.
#[test]
fn test_cascade_fixture_themes_the_schema_root() {
    let map = cascade_map();
    let tree = build_mindmap_tree(&map);
    assert_eq!(
        glyph_area_of(&tree.tree, tree.arena_id_for("0").unwrap()).background_color,
        Some([0xa1, 0, 0, 255])
    );
    assert_eq!(frame_color_of(&map, "0"), "#a20000");
}

/// …and node `1`, whose schema says `starts_at_root: false`, is the
/// transparent root: its own `style` stands, and its child at
/// `level: 1` picks up `groups[0]`.
#[test]
fn test_cascade_fixture_honors_starts_at_root_false() {
    let map = cascade_map();
    let tree = build_mindmap_tree(&map);
    assert_eq!(
        glyph_area_of(&tree.tree, tree.arena_id_for("1").unwrap()).background_color,
        Some([0x11, 0x11, 0x11, 255]),
        "the transparent schema root keeps style.background_color"
    );
    assert_eq!(
        glyph_area_of(&tree.tree, tree.arena_id_for("1.0").unwrap()).background_color,
        Some([0xa1, 0, 0, 255]),
        "its child shifts onto groups[0]"
    );
}

/// Node `0.0.0` sits at `level: 9` against a three-group palette.
#[test]
fn test_cascade_fixture_clamps_an_out_of_range_level() {
    let map = cascade_map();
    let tree = build_mindmap_tree(&map);
    assert_eq!(
        glyph_area_of(&tree.tree, tree.arena_id_for("0.0.0").unwrap()).background_color,
        Some([0xc1, 0, 0, 255]),
        "level 9 clamps to the last of three groups"
    );
}

#[test]
fn test_cascade_fixture_leaves_the_unthemed_node_on_its_style() {
    let map = cascade_map();
    let tree = build_mindmap_tree(&map);
    assert_eq!(
        glyph_area_of(&tree.tree, tree.arena_id_for("2").unwrap()).background_color,
        Some([0x11, 0x11, 0x11, 255])
    );
}

/// The fixture's edges, per source node: `0 -> 0.0` and `1 -> 1.0`
/// carry `connections_colored`, `0.0 -> 0.0.0` inherits the flag
/// from `0.0` (also on), and `2 -> 0` leaves from an unthemed node
/// so it keeps `edge.color`.
#[test]
fn test_cascade_fixture_edge_colors_follow_their_source_nodes() {
    let map = cascade_map();
    let projected = project(&map, 1.0);
    let color_of = |from: &str, to: &str| -> String {
        projected
            .connection_elements
            .iter()
            .find(|c| c.edge_key.from_id == from && c.edge_key.to_id == to)
            .unwrap_or_else(|| panic!("edge {from} -> {to} must project"))
            .color
            .clone()
    };
    assert_eq!(color_of("0", "0.0"), "#a20000", "source node 0 is at groups[0]");
    assert_eq!(
        color_of("0.0", "0.0.0"),
        "#b20000",
        "source node 0.0 is at groups[1]"
    );
    assert_eq!(
        color_of("1", "1.0"),
        "#909090",
        "source node 1 is the transparent schema root — no group, so no theme stroke"
    );
    assert_eq!(color_of("2", "0"), "#909090", "source node 2 carries no schema");
}
