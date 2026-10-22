// SPDX-License-Identifier: MPL-2.0

//! Canvas-role signature tests — the two questions the border and
//! portal dispatchers ask before touching a registered tree.
//!
//! `*_structure_signature` answers *can a mutator align against
//! the registered arena?* and is a deliberate subset of the data.
//! `*_content_signature` answers *is there anything left for that
//! mutator to write?* and must miss nothing: on a match
//! `CanvasFrame::update_border_tree` /
//! `CanvasFrame::update_portal_tree` do no work at all, so a field
//! the projection reads and the signature does not is a stale
//! frame on screen with nothing to report it.
//!
//! The two signatures are therefore tested in opposite
//! directions. Both corpora below are **paired with the tree the
//! signature guards**: every row asserts first that the change it
//! makes actually reaches the rendered tree — a row that does not
//! is a fixture that proves nothing, and says so instead of
//! passing — and then that the signature moved with it. Nothing
//! here computes its expectation with the hasher under test:
//! [`trees_draw_identically`] is built from `GlyphArea`'s own
//! `PartialEq` plus `ColorFontRegions::same_content`, never from
//! `hash_content`.

use std::collections::HashMap;

use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::{BranchChannel, Tree};
use crate::mindmap::model::{
    ColorGroup, ColorOverrides, ColorSchema, CustomBorderGlyphs, GlyphBorderConfig, MindMap, Palette,
    PortalEndpointState,
};
use crate::mindmap::scene_cache::EdgeKey;
use crate::mindmap::tree_builder::{
    border_content_signature, border_node_data, border_structure_signature, build_border_tree_from_nodes,
    build_portal_tree_from_pairs, portal_content_signature, portal_pair_data, portal_structure_signature,
    BorderChromeOverrides, BorderConfigEditsView, BorderNodeData, BorderPreview, BorderPreviewTargetRef,
    EditView, PortalColorPreview, PortalPairData, PortalTextEditOverride, SelectedPortalLabel,
};

use super::fixtures::{sized_node, synthetic_map, synthetic_node, synthetic_portal_edge};

/// Whether two built canvas trees would draw identically: same
/// shape, same channels, and per `GlyphArea` every field —
/// *including* the per-span colors `PartialEq` on a
/// `ColorFontRegions` cannot see, since its element identity is
/// the range alone.
///
/// Deliberately independent of the code under test. A comparison
/// derived from `hash_content` would agree with the signature by
/// construction and prove nothing (issue #138, shape 6: "two
/// derivations sharing a source"). `GlyphArea::eq` and
/// `ColorFontRegions::same_content` predate this work and answer
/// the same question by other means.
///
/// The one field neither this comparison nor the signature reads
/// is `hitbox`, which both builders leave empty — pinned by
/// `test_border_and_portal_areas_carry_no_hitbox`.
fn trees_draw_identically(a: &Tree<GfxElement, GfxMutator>, b: &Tree<GfxElement, GfxMutator>) -> bool {
    let a_nodes: Vec<_> = a.root.descendants(&a.arena).collect();
    let b_nodes: Vec<_> = b.root.descendants(&b.arena).collect();
    if a_nodes.len() != b_nodes.len() {
        return false;
    }
    for (a_id, b_id) in a_nodes.iter().zip(b_nodes.iter()) {
        let a_el = a.arena.get(*a_id).unwrap().get();
        let b_el = b.arena.get(*b_id).unwrap().get();
        if a_el.channel() != b_el.channel() {
            return false;
        }
        match (a_el.glyph_area(), b_el.glyph_area()) {
            (None, None) => {}
            (Some(a_area), Some(b_area)) => {
                if a_area != b_area || !a_area.regions.same_content(&b_area.regions) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

// -----------------------------------------------------------------
// Borders
// -----------------------------------------------------------------

/// The border corpus's starting state, deliberately rich enough
/// that every axis below has something to move.
///
/// Four framed nodes, because no single one can carry every axis:
/// `a` has a fully-authored `GlyphBorderConfig` with an explicit
/// color and custom glyphs, so the color / pattern / corner /
/// preset / size axes have values to change; `d` binds a palette
/// instead, since a bound palette *replaces* the flat color in the
/// emitted regions and would swallow a color edit; `b` takes its
/// frame color through a theme variable, so the variable itself is
/// an axis; `c` is a child of `b`, so folding `b` is an axis.
///
/// A fixture whose content never varies in the dimension a
/// signature is supposed to cover proves nothing about it — the
/// blind spot that shipped twice in this issue's earlier PRs — so
/// the shape of this map is the test.
fn base_border_map() -> MindMap {
    let mut a = sized_node("a", 10.0, 20.0, 160.0, 90.0, true);
    a.style.frame_color = "#ff8800".into();
    a.style.border = Some(GlyphBorderConfig {
        preset: "custom".into(),
        font: None,
        font_size_pt: 12.0,
        color: Some("#ff8800".into()),
        glyphs: Some(CustomBorderGlyphs {
            top: "─".into(),
            bottom: "─".into(),
            left: "│".into(),
            right: "│".into(),
            top_left: "┌".into(),
            top_right: "┐".into(),
            bottom_left: "└".into(),
            bottom_right: "┘".into(),
        }),
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    });

    let mut b = sized_node("b", 400.0, 20.0, 120.0, 60.0, true);
    b.style.frame_color = "var(--frame)".into();

    let c = synthetic_node("c", Some("b"), 400.0, 200.0);

    let mut d = sized_node("d", 700.0, 20.0, 140.0, 70.0, true);
    d.style.border = Some(GlyphBorderConfig {
        preset: "light".into(),
        font: None,
        font_size_pt: 12.0,
        color: Some("#ffffff".into()),
        glyphs: None,
        padding: 0.0,
        color_palette: Some("warm".into()),
        color_palette_field: Some("frame".into()),
    });

    let mut f = sized_node("f", 1000.0, 20.0, 100.0, 50.0, true);
    f.color_schema = Some(ColorSchema {
        palette: "warm".into(),
        level: 0,
        starts_at_root: true,
        connections_colored: false,
        overrides: ColorOverrides::default(),
    });

    let mut map = synthetic_map(vec![a, b, c, d, f], vec![]);
    map.canvas
        .theme_variables
        .insert("--frame".into(), "#1188ff".into());
    map.palettes.insert(
        "warm".into(),
        Palette {
            groups: vec![
                ColorGroup {
                    background: "#100000".into(),
                    frame: "#ff0000".into(),
                    text: "#ffcccc".into(),
                    title: "#ffeeee".into(),
                },
                ColorGroup {
                    background: "#001000".into(),
                    frame: "#00ff00".into(),
                    text: "#ccffcc".into(),
                    title: "#eeffee".into(),
                },
            ],
        },
    );
    map
}

/// Run the border data pass the way `CanvasFrame::update_border_tree`
/// does, with the two chrome overrides the app threads.
fn border_nodes(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    node_edit_for: Option<&str>,
) -> Vec<BorderNodeData> {
    border_node_data(
        map,
        offsets,
        BorderChromeOverrides {
            preview: None,
            node_edit_for,
        },
        &map.fold_hidden_set(),
    )
}

/// Borrow node `a`'s authored border config — the flat-color one.
fn border_cfg(map: &mut MindMap) -> &mut GlyphBorderConfig {
    node_border_cfg(map, "a")
}

/// Borrow node `d`'s authored border config — the palette-bound
/// one. Kept apart from [`border_cfg`] because a bound palette
/// replaces the flat color in the emitted regions, so the two
/// cannot be exercised on the same node.
fn palette_cfg(map: &mut MindMap) -> &mut GlyphBorderConfig {
    node_border_cfg(map, "d")
}

fn node_border_cfg<'a>(map: &'a mut MindMap, id: &str) -> &'a mut GlyphBorderConfig {
    map.nodes.get_mut(id).unwrap().style.border.as_mut().unwrap()
}

/// Borrow node `f`'s `color_schema` — the palette-theme tier of
/// the frame-color cascade.
fn theme_schema(map: &mut MindMap) -> &mut ColorSchema {
    map.nodes.get_mut("f").unwrap().color_schema.as_mut().unwrap()
}

/// One corpus row per input the border data pass reads, each
/// carrying the node data that input's edit produces.
///
/// The list is the enumeration the signature's completeness is
/// argued from, so it is written out rather than generated, and
/// each row is checked against the tree before it is checked
/// against the signature.
///
/// **What "every input" means here, spelled out so the claim can
/// be checked rather than believed.** `border_node_data`'s
/// parameters are `(map, offsets, overrides, hidden_set)`, and the
/// rows cover: from `map` — `canvas.theme_variables`,
/// `canvas.default_border`, `palettes`, `node_frame_theme_tier`
/// (both its rungs), and per node `style.show_frame`,
/// `style.shape`, `style.frame_color`, `style.border`,
/// `position`, `size`, `zoom_window`, plus membership of
/// `map.nodes` itself; from `offsets` — a drag delta; from
/// `overrides` — `node_edit_for` and all three shapes of
/// `preview`; and `hidden_set` through a fold.
///
/// Two deliberate non-rows. `BorderPreview.edits` has sixteen
/// fields, and every one of them is folded by `apply_view_to_slot`
/// into the same `GlyphBorderConfig` whose axes the direct-config
/// rows above already vary — two are exercised here to prove the
/// preview *path* reaches the tree, not to re-cover the config.
/// And the `Sections` / `CanvasSectionFrame*` preview targets
/// belong to `build_section_frames`; `BorderChromeOverrides`
/// documents that this pass ignores them, so a row would assert
/// that nothing happens.
fn border_rows() -> Vec<(&'static str, Vec<BorderNodeData>)> {
    let mut rows: Vec<(&'static str, Vec<BorderNodeData>)> = Vec::new();
    let none = HashMap::new();

    let mut m = base_border_map();
    m.nodes.get_mut("a").unwrap().position.x += 7.0;
    rows.push(("node position", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    m.nodes.get_mut("a").unwrap().size.width += 13.0;
    rows.push(("node size", border_nodes(&m, &none, None)));

    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (5.0, -3.0));
    rows.push(("drag offset", border_nodes(&base_border_map(), &offsets, None)));

    let mut m = base_border_map();
    border_cfg(&mut m).color = Some("#00ff88".into());
    rows.push(("authored border color", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    m.nodes.get_mut("b").unwrap().style.frame_color = "#00ff00".into();
    rows.push(("node frame_color fallback", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    m.canvas
        .theme_variables
        .insert("--frame".into(), "#992200".into());
    rows.push(("theme variable value", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    border_cfg(&mut m).glyphs.as_mut().unwrap().top = "═".into();
    rows.push(("side pattern glyph", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    border_cfg(&mut m).glyphs.as_mut().unwrap().top_left = "╔".into();
    rows.push(("corner glyph", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    let cfg = border_cfg(&mut m);
    cfg.preset = "heavy".into();
    cfg.glyphs = None;
    rows.push(("preset swap", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    border_cfg(&mut m).font_size_pt = 12.5;
    rows.push(("border font size", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    palette_cfg(&mut m).color_palette = None;
    rows.push(("palette unbound", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    palette_cfg(&mut m).color_palette_field = Some("text".into());
    rows.push(("palette field", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    m.palettes.get_mut("warm").unwrap().groups[0].frame = "#0000ff".into();
    rows.push(("palette group color", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    let node_a = m.nodes.get_mut("a").unwrap();
    node_a.min_zoom_to_render = Some(0.5);
    node_a.max_zoom_to_render = Some(2.0);
    rows.push(("node zoom window", border_nodes(&m, &none, None)));

    rows.push((
        "NodeEdit dimming",
        border_nodes(&base_border_map(), &none, Some("a")),
    ));

    // The canvas-wide rung of the border cascade: nodes `b` and
    // `c` carry no per-node config, so this is the layer their
    // style resolves from.
    let mut m = base_border_map();
    m.canvas.default_border = Some(GlyphBorderConfig {
        preset: "heavy".into(),
        font: None,
        font_size_pt: 15.0,
        color: Some("#33ddaa".into()),
        glyphs: None,
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    });
    rows.push(("canvas default border", border_nodes(&m, &none, None)));

    // The palette-theme rung, reached only through a node's
    // `color_schema`. Two rows because it has two tiers of its own:
    // the per-node channel override, and the palette group the
    // schema's level selects.
    let mut m = base_border_map();
    theme_schema(&mut m).overrides.frame = Some("#7700bb".into());
    rows.push(("node theme frame override", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    theme_schema(&mut m).level = 1;
    rows.push(("node theme palette level", border_nodes(&m, &none, None)));

    // `BorderChromeOverrides.preview` — the staged `border preview
    // …` edit, and the border half of the very interaction this
    // whole design is justified by. Three rows: a per-node target,
    // the canvas-default target, and `force_show_frame`, which
    // renders a preview against a node whose committed
    // `show_frame` is off and so changes the framed-node set.
    let preview_ids = ["a".to_string()];
    let m = base_border_map();
    rows.push((
        "border preview on a node",
        border_node_data(
            &m,
            &none,
            BorderChromeOverrides {
                preview: Some(BorderPreview {
                    target: BorderPreviewTargetRef::Nodes(&preview_ids),
                    edits: BorderConfigEditsView {
                        color: EditView::Set("#eebb00"),
                        ..BorderConfigEditsView::default()
                    },
                    force_show_frame: false,
                }),
                node_edit_for: None,
            },
            &m.fold_hidden_set(),
        ),
    ));

    let m = base_border_map();
    rows.push((
        "border preview on the canvas default",
        border_node_data(
            &m,
            &none,
            BorderChromeOverrides {
                preview: Some(BorderPreview {
                    target: BorderPreviewTargetRef::CanvasDefault,
                    edits: BorderConfigEditsView {
                        preset: EditView::Set("heavy"),
                        ..BorderConfigEditsView::default()
                    },
                    force_show_frame: false,
                }),
                node_edit_for: None,
            },
            &m.fold_hidden_set(),
        ),
    ));

    let frameless_ids = ["g".to_string()];
    let mut m = base_border_map();
    let mut g = sized_node("g", 1300.0, 20.0, 90.0, 45.0, false);
    g.style.frame_color = "#445566".into();
    m.nodes.insert("g".into(), g);
    rows.push((
        "border preview force_show_frame",
        border_node_data(
            &m,
            &none,
            BorderChromeOverrides {
                preview: Some(BorderPreview {
                    target: BorderPreviewTargetRef::Nodes(&frameless_ids),
                    edits: BorderConfigEditsView {
                        color: EditView::Set("#eebb00"),
                        ..BorderConfigEditsView::default()
                    },
                    force_show_frame: true,
                }),
                node_edit_for: None,
            },
            &m.fold_hidden_set(),
        ),
    ));

    let mut m = base_border_map();
    m.nodes.get_mut("b").unwrap().style.show_frame = false;
    rows.push(("show_frame toggle", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    m.nodes.get_mut("b").unwrap().folded = true;
    rows.push(("folded ancestor", border_nodes(&m, &none, None)));

    let mut m = base_border_map();
    m.nodes.get_mut("b").unwrap().style.shape = "ellipse".into();
    rows.push(("non-rectangular shape", border_nodes(&m, &none, None)));

    rows
}

/// Every input the border data pass reads, when it moves the
/// rendered tree, moves the content signature with it.
///
/// This is the assertion the `InPlaceMutator` skip rests on. Its
/// failing input is any edit that reaches a glyph run without
/// reaching the hash — drop `zoom_visibility` from
/// `border_content_signature`, or `side_patterns` from
/// `BorderStyle::hash_content`, and the matching row goes red.
/// Left unguarded, the dispatcher would decide "no work needed"
/// and the screen would keep the previous frame's border.
///
/// The precondition on each row is not ceremony: it is the guard
/// against the corpus blind spot this issue has already shipped
/// twice, where a fixture that never varies in the dimension under
/// test leaves the whole suite green.
#[test]
fn test_border_content_signature_moves_for_every_change_the_tree_shows() {
    let base_map = base_border_map();
    let base = border_nodes(&base_map, &HashMap::new(), None);
    let base_tree = build_border_tree_from_nodes(&base);
    let base_sig = border_content_signature(&base);

    assert_eq!(
        base_sig,
        border_content_signature(&border_nodes(&base_border_map(), &HashMap::new(), None)),
        "the signature must be a pure function of the node data — a signature that \
         differs between two identical passes never matches and the skip never fires"
    );

    for (label, nodes) in border_rows() {
        let tree = build_border_tree_from_nodes(&nodes);
        assert!(
            !trees_draw_identically(&base_tree, &tree),
            "corpus row {label:?} leaves the rendered border tree identical, so it cannot \
             prove anything about the signature — fix the row, not the assertion"
        );
        assert_ne!(
            base_sig,
            border_content_signature(&nodes),
            "border_content_signature is blind to {label:?}: the tree it guards renders \
             differently and the signature did not move, so update_border_tree would skip \
             the update and leave stale glyphs on screen"
        );
    }
}

/// Two framed sets of the same size, laid out identically, under
/// different node ids: the rendered tree cannot tell them apart,
/// and the structural signature must.
///
/// This is the half a corpus of add / remove / fold edits cannot
/// reach, because each of those also changes the count. What is at
/// stake is not glyphs — the glyphs are the same — but *which node
/// a mutator channel belongs to*: `align_child_walks` pairs mutator
/// children with target children by ascending channel, so a per-node
/// Void that changed owners without the signature moving would write
/// one node's runs into another's.
///
/// Failing input: drop `node_id` from `border_structure_signature`.
/// The tree-equality assertion below is the precondition that makes
/// that the only thing this test can be measuring.
#[test]
fn test_border_structure_signature_moves_when_ids_change_at_a_fixed_count() {
    let base_map = base_border_map();
    let mut renamed_map = base_border_map();
    let mut moved = renamed_map.nodes.remove("d").unwrap();
    moved.id = "e".into();
    renamed_map.nodes.insert("e".into(), moved);

    let base = border_nodes(&base_map, &HashMap::new(), None);
    let renamed = border_nodes(&renamed_map, &HashMap::new(), None);
    assert_eq!(base.len(), renamed.len(), "the framed-node count must not move");
    assert!(
        trees_draw_identically(
            &build_border_tree_from_nodes(&base),
            &build_border_tree_from_nodes(&renamed)
        ),
        "the two maps must render identically, or this test is measuring a glyph change \
         rather than an identity change"
    );
    assert_ne!(
        border_structure_signature(&base),
        border_structure_signature(&renamed),
        "the framed-node identities changed under a fixed count, so the structural \
         signature must move and force a full rebuild rather than let a mutator write \
         one node's runs into another's"
    );
}

/// `font_name` is the one style axis carried in the content
/// signature that the corpus above cannot pair with a tree
/// difference, and it is tested apart rather than quietly dropped.
///
/// It reaches the runs only through `border_run_specs`'s per-glyph
/// ink measurement, and *whether* that measurement moves depends
/// on the glyphs as much as on the face: for this fixture's
/// box-drawing borders it does not, so every run lands where it
/// did. Which face the shaper ends up using for them is not
/// asserted here — the observable is, below. Hashing the name
/// anyway is the over-inclusive direction: it can cost a redundant
/// update, never a skipped one.
///
/// Both halves are asserted, so the paragraph above reproduces
/// rather than being taken on trust. If a font or glyph change
/// ever makes the two trees differ, the first assertion fails and
/// says to move this case into [`border_rows`], where the corpus
/// would then cover it.
#[test]
fn test_border_content_signature_covers_font_name_the_tree_cannot_show() {
    let mut m = base_border_map();
    border_cfg(&mut m).font = Some("Liberation Mono".into());
    let named = border_nodes(&m, &HashMap::new(), None);
    let base = border_nodes(&base_border_map(), &HashMap::new(), None);

    assert!(
        trees_draw_identically(
            &build_border_tree_from_nodes(&base),
            &build_border_tree_from_nodes(&named)
        ),
        "the authored border font now moves the rendered tree — this case is no longer a \
         can't-show-it one and belongs in the tree-paired corpus"
    );
    assert_ne!(
        border_content_signature(&base),
        border_content_signature(&named),
        "font_name change must move the content signature"
    );
}

/// The structural signature is a *subset* on purpose, and both
/// halves of that are load-bearing.
///
/// It must **not** move for the continuous interactions — drag,
/// recolor, palette edit, dimming — because a structural mismatch
/// sends the role down the full-rebuild arm, and reallocating the
/// border arena on every frame of a color-picker hover is exactly
/// what the §B2 in-place path exists to avoid. It must move for
/// anything that changes the channel layout the mutator aligns
/// against, or the mutator writes into the wrong node's runs.
///
/// Failing input for the first half: fold any content field into
/// `border_structure_signature`. The second half is the *count*
/// half — every shape-changing row here removes a framed node —
/// and the identity half it cannot see has its own test below,
/// `test_border_structure_signature_moves_when_ids_change_at_a_fixed_count`.
#[test]
fn test_border_structure_signature_tracks_shape_and_ignores_content() {
    let base = border_nodes(&base_border_map(), &HashMap::new(), None);
    let base_sig = border_structure_signature(&base);

    // Rows whose edits leave the framed-node set alone. Named
    // against `border_rows` above, which is where their node data
    // is built.
    let content_only = [
        "node position",
        "node size",
        "drag offset",
        "authored border color",
        "node frame_color fallback",
        "theme variable value",
        "side pattern glyph",
        "corner glyph",
        "preset swap",
        "border font size",
        "palette unbound",
        "palette field",
        "palette group color",
        "node zoom window",
        "NodeEdit dimming",
        "canvas default border",
        "node theme frame override",
        "node theme palette level",
        "border preview on a node",
        "border preview on the canvas default",
    ];
    // `force_show_frame` is the one preview axis that is structural:
    // it frames a node whose committed `show_frame` is off, which is
    // a node the mutator has no channel for.
    let shape_changing = [
        "show_frame toggle",
        "folded ancestor",
        "non-rectangular shape",
        "border preview force_show_frame",
    ];

    let rows = border_rows();
    assert_eq!(
        rows.len(),
        content_only.len() + shape_changing.len(),
        "every corpus row must be classified as one or the other — an unclassified row \
         is a dimension this test silently stops covering"
    );

    for (label, nodes) in rows {
        let sig = border_structure_signature(&nodes);
        if content_only.contains(&label) {
            assert_eq!(
                base_sig, sig,
                "{label:?} changes content only, so the structural signature must hold \
                 still and keep the in-place mutator arm reachable"
            );
        } else {
            assert!(
                shape_changing.contains(&label),
                "corpus row {label:?} is in neither classification list"
            );
            assert_ne!(
                base_sig, sig,
                "{label:?} changes which nodes are framed, so the structural signature \
                 must move and force a full rebuild"
            );
        }
    }
}

// -----------------------------------------------------------------
// Portals
// -----------------------------------------------------------------

/// The portal corpus's starting state.
///
/// Three portal-mode edges, because the edge color cascade
/// shadows: a rung is only reachable on an edge whose higher rungs
/// are unset.
///
/// `a ↔ b` carries fully-authored endpoint state on both ends
/// (color, text, text color, text size, perimeter position,
/// perpendicular slide) and a `glyph_connection.color` naming a
/// theme variable, so every per-endpoint axis and the variable
/// itself have values to move. `b ↔ q` hangs off a child of `p`,
/// so folding `p` drops a pair and moves the *structure*; it
/// carries no `glyph_connection.color`, which is what leaves node
/// `b`'s `connections_colored` theme tier reachable on it.
/// `r ↔ t` carries no `glyph_connection` at all, which is the only
/// way down to `canvas.default_connection`.
fn base_portal_map() -> MindMap {
    let a = sized_node("a", 0.0, 0.0, 80.0, 40.0, false);
    let b = sized_node("b", 400.0, 0.0, 80.0, 40.0, false);
    let p = sized_node("p", 0.0, 300.0, 80.0, 40.0, false);
    let q = synthetic_node("q", Some("p"), 400.0, 300.0);

    let mut edge = synthetic_portal_edge("a", "b", "#ffffff");
    if let Some(cfg) = edge.glyph_connection.as_mut() {
        cfg.color = Some("var(--portal)".into());
    }
    edge.portal_from = Some(PortalEndpointState {
        color: Some("#ff0000".into()),
        text: Some("from".into()),
        text_color: Some("#00ff00".into()),
        text_font_size_pt: Some(11.0),
        border_t: Some(0.5),
        perpendicular_offset: Some(4.0),
        ..PortalEndpointState::default()
    });
    edge.portal_to = Some(PortalEndpointState {
        text: Some("to".into()),
        ..PortalEndpointState::default()
    });

    // No `glyph_connection.color`, so the themed rung below it is
    // reachable — `b` is this edge's source node.
    let second = synthetic_portal_edge("b", "q", "#ffffff");

    // No `glyph_connection` at all, so `resolved_for` falls through
    // to `canvas.default_connection`.
    let mut third = synthetic_portal_edge("r", "t", "#ffffff");
    third.glyph_connection = None;

    let mut b_node = b;
    b_node.color_schema = Some(ColorSchema {
        palette: "warm".into(),
        level: 0,
        starts_at_root: true,
        connections_colored: true,
        overrides: ColorOverrides {
            frame: Some("#22ccaa".into()),
            ..ColorOverrides::default()
        },
    });

    let r = sized_node("r", 0.0, 600.0, 80.0, 40.0, false);
    let t = sized_node("t", 400.0, 600.0, 80.0, 40.0, false);

    let mut map = synthetic_map(vec![a, b_node, p, q, r, t], vec![edge, second, third]);
    map.canvas
        .theme_variables
        .insert("--portal".into(), "#8800ff".into());
    map
}

/// The steady-state portal pass: no selection, no preview, no
/// inline edit, camera at 1.0 — what a rebuild with nothing
/// pointed at produces.
fn portal_pairs(map: &MindMap) -> Vec<PortalPairData> {
    portal_pair_data(
        map,
        &HashMap::new(),
        None,
        None,
        None,
        None,
        1.0,
        &map.fold_hidden_set(),
    )
}

/// Borrow the authored `a ↔ b` edge's from-side endpoint state.
fn from_state(map: &mut MindMap) -> &mut PortalEndpointState {
    map.edges[0].portal_from.as_mut().unwrap()
}

/// One corpus row per input the portal data pass reads, each
/// carrying the pair data that input's edit produces. Same
/// contract as [`border_rows`]: each row is checked against the
/// tree before it is checked against the signature.
///
/// **What "every input" means here.** `portal_pair_data`'s
/// parameters are `(map, offsets, selected_edge,
/// selected_portal_label, color_preview, portal_text_edit,
/// camera_zoom, hidden_set)`, and each of the six non-map ones has
/// a row of its own. From `map` the rows cover:
/// `canvas.theme_variables`, `canvas.default_connection`,
/// `edge_theme_stroke_color`, per edge `display_mode`, `visible`,
/// `glyph_connection` (body / size / color), the edge zoom window,
/// and per endpoint `color`, `text`, `text_color`,
/// `text_font_size_pt`, the text font clamps, `border_t`,
/// `perpendicular_offset` and the endpoint zoom window; plus the
/// owner and partner nodes' `position` and `size`, and membership
/// through a fold.
///
/// One deliberate non-row: `ResolvedPortalStyle.font`. The
/// resolver produces it, but neither `layout_portal_label` nor the
/// `GlyphArea` the projection builds carries a font pin, so no
/// edit to it can reach the tree — the same shape as the border
/// side's `font_name`, and pinned there by
/// `test_border_content_signature_covers_font_name_the_tree_cannot_show`.
/// The three cascade rungs above it are rows, so the resolver
/// itself is exercised.
fn portal_rows() -> Vec<(&'static str, Vec<PortalPairData>)> {
    let mut rows: Vec<(&'static str, Vec<PortalPairData>)> = Vec::new();
    let key = EdgeKey::new("a", "b", "cross_link");
    let none = HashMap::new();

    let mut m = base_portal_map();
    m.nodes.get_mut("a").unwrap().position.y += 11.0;
    rows.push(("owner node position", portal_pairs(&m)));

    let mut m = base_portal_map();
    m.nodes.get_mut("a").unwrap().size.width += 17.0;
    rows.push(("owner node size", portal_pairs(&m)));

    let mut m = base_portal_map();
    m.nodes.get_mut("b").unwrap().position.y += 200.0;
    rows.push(("partner node position", portal_pairs(&m)));

    let m = base_portal_map();
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (9.0, 6.0));
    rows.push((
        "drag offset",
        portal_pair_data(&m, &offsets, None, None, None, None, 1.0, &m.fold_hidden_set()),
    ));

    let mut m = base_portal_map();
    from_state(&mut m).text = Some("edited".into());
    rows.push(("endpoint text", portal_pairs(&m)));

    let mut m = base_portal_map();
    from_state(&mut m).color = Some("#00ffff".into());
    rows.push(("endpoint icon color", portal_pairs(&m)));

    let mut m = base_portal_map();
    from_state(&mut m).text_color = Some("#ffff00".into());
    rows.push(("endpoint text color", portal_pairs(&m)));

    let mut m = base_portal_map();
    from_state(&mut m).text_font_size_pt = Some(19.0);
    rows.push(("endpoint text font size", portal_pairs(&m)));

    let mut m = base_portal_map();
    from_state(&mut m).border_t = Some(2.5);
    rows.push(("endpoint border_t", portal_pairs(&m)));

    let mut m = base_portal_map();
    from_state(&mut m).perpendicular_offset = Some(40.0);
    rows.push(("endpoint perpendicular offset", portal_pairs(&m)));

    let mut m = base_portal_map();
    let state = from_state(&mut m);
    state.min_zoom_to_render = Some(0.5);
    state.max_zoom_to_render = Some(3.0);
    rows.push(("endpoint zoom window", portal_pairs(&m)));

    let mut m = base_portal_map();
    if let Some(cfg) = m.edges[0].glyph_connection.as_mut() {
        cfg.color = Some("#123456".into());
    }
    rows.push(("edge glyph color", portal_pairs(&m)));

    let mut m = base_portal_map();
    m.canvas
        .theme_variables
        .insert("--portal".into(), "#224466".into());
    rows.push(("theme variable value", portal_pairs(&m)));

    let mut m = base_portal_map();
    if let Some(cfg) = m.edges[0].glyph_connection.as_mut() {
        cfg.body = "\u{25CF}".into();
    }
    rows.push(("marker glyph", portal_pairs(&m)));

    let mut m = base_portal_map();
    if let Some(cfg) = m.edges[0].glyph_connection.as_mut() {
        cfg.font_size_pt = 24.0;
    }
    rows.push(("marker font size", portal_pairs(&m)));

    // 0.25 rather than a zoom-in: the marker's canvas-space size is
    // `clamp(base * zoom, min_font_size_pt, max_font_size_pt) / zoom`,
    // so inside the clamp window zoom cancels out exactly and the
    // tree does not move at all. It reaches the glyphs only where
    // the clamp bites — here `16 * 0.25 = 4` is lifted to the 8 pt
    // floor. A row at 2.5 looked like a covered dimension and was
    // caught by the precondition below.
    let m = base_portal_map();
    rows.push((
        "camera zoom past the font clamp",
        portal_pair_data(&m, &none, None, None, None, None, 0.25, &m.fold_hidden_set()),
    ));

    let m = base_portal_map();
    rows.push((
        "edge selection highlight",
        portal_pair_data(
            &m,
            &none,
            Some(("a", "b", "cross_link")),
            None,
            None,
            None,
            1.0,
            &m.fold_hidden_set(),
        ),
    ));

    let m = base_portal_map();
    rows.push((
        "portal-label selection highlight",
        portal_pair_data(
            &m,
            &none,
            None,
            Some(SelectedPortalLabel {
                edge_key: &key,
                endpoint_node_id: "a",
            }),
            None,
            None,
            1.0,
            &m.fold_hidden_set(),
        ),
    ));

    let m = base_portal_map();
    rows.push((
        "color-picker preview",
        portal_pair_data(
            &m,
            &none,
            None,
            None,
            Some(PortalColorPreview {
                edge_key: &key,
                color: "#abcdef",
            }),
            None,
            1.0,
            &m.fold_hidden_set(),
        ),
    ));

    let m = base_portal_map();
    rows.push((
        "inline text-edit buffer",
        portal_pair_data(
            &m,
            &none,
            None,
            None,
            None,
            Some(PortalTextEditOverride {
                edge_key: &key,
                endpoint_node_id: "a",
                buffer: "typing",
            }),
            1.0,
            &m.fold_hidden_set(),
        ),
    ));

    // The themed rung of the edge color cascade, reachable on the
    // second edge because it names no `glyph_connection.color`.
    // Its source node `b` is the one carrying `connections_colored`.
    let mut m = base_portal_map();
    m.nodes
        .get_mut("b")
        .unwrap()
        .color_schema
        .as_mut()
        .unwrap()
        .overrides
        .frame = Some("#ff44aa".into());
    rows.push(("edge theme stroke color", portal_pairs(&m)));

    // The canvas-wide rung, reachable on the third edge because it
    // carries no `glyph_connection` at all.
    let mut m = base_portal_map();
    m.canvas.default_connection = Some(crate::mindmap::model::GlyphConnectionConfig {
        body: "\u{25A0}".into(),
        font_size_pt: 21.0,
        ..crate::mindmap::model::GlyphConnectionConfig::default()
    });
    rows.push(("canvas default connection", portal_pairs(&m)));

    // The edge-level zoom window, which an endpoint's own window
    // *replaces* rather than intersects — so it is only observable
    // on an edge whose endpoints carry none. That is the second
    // edge.
    let mut m = base_portal_map();
    m.edges[1].min_zoom_to_render = Some(0.4);
    m.edges[1].max_zoom_to_render = Some(4.0);
    rows.push(("edge zoom window", portal_pairs(&m)));

    // The per-endpoint text font clamps, which bite only when they
    // cross the authored size — 11 pt here, lifted to 20.
    let mut m = base_portal_map();
    from_state(&mut m).text_min_font_size_pt = Some(20.0);
    rows.push(("endpoint text font clamp", portal_pairs(&m)));

    let mut m = base_portal_map();
    m.edges[1].visible = false;
    rows.push(("edge visibility", portal_pairs(&m)));

    let mut m = base_portal_map();
    m.edges[1].display_mode = None;
    rows.push(("edge display mode", portal_pairs(&m)));

    let mut m = base_portal_map();
    m.nodes.get_mut("p").unwrap().folded = true;
    rows.push(("folded endpoint", portal_pairs(&m)));

    rows
}

/// Every input the portal data pass reads, when it moves the
/// rendered tree, moves the content signature with it.
///
/// The row that matters most is `endpoint icon color`, and it is
/// the reason this signature cannot be built on `GlyphArea`'s own
/// `Hash`. That impl has to agree with a `PartialEq` whose region
/// identity is the span range, so a recolored marker hashes to the
/// value it replaced — swap `hash_content` for `hash` in
/// `portal_content_signature` and this test goes red on exactly
/// the rows a color-picker hover produces, which is the
/// interaction the portal role exists to keep cheap.
#[test]
fn test_portal_content_signature_moves_for_every_change_the_tree_shows() {
    let base = portal_pairs(&base_portal_map());
    let base_tree = build_portal_tree_from_pairs(&base).tree;
    let base_sig = portal_content_signature(&base);

    assert_eq!(
        base_sig,
        portal_content_signature(&portal_pairs(&base_portal_map())),
        "the signature must be a pure function of the pair data — a signature that \
         differs between two identical passes never matches and the skip never fires"
    );

    for (label, pairs) in portal_rows() {
        let tree = build_portal_tree_from_pairs(&pairs).tree;
        assert!(
            !trees_draw_identically(&base_tree, &tree),
            "corpus row {label:?} leaves the rendered portal tree identical, so it cannot \
             prove anything about the signature — fix the row, not the assertion"
        );
        assert_ne!(
            base_sig,
            portal_content_signature(&pairs),
            "portal_content_signature is blind to {label:?}: the tree it guards renders \
             differently and the signature did not move, so update_portal_tree would skip \
             the update and leave a stale marker on screen"
        );
    }
}

/// The portal structural signature holds still for everything a
/// mutator can write in place, and moves for everything that
/// changes which pair owns which channel.
///
/// The second half is not only about glyphs. `PortalHitIndex`
/// resolves a click by indexing the pair channel positionally, so
/// a pair sequence that changed without the signature moving would
/// name the wrong edge for a click that landed on a portal.
#[test]
fn test_portal_structure_signature_tracks_shape_and_ignores_content() {
    let base = portal_pairs(&base_portal_map());
    let base_sig = portal_structure_signature(&base);

    let shape_changing = ["edge visibility", "edge display mode", "folded endpoint"];
    let rows = portal_rows();
    let content_only_count = rows.len() - shape_changing.len();
    let mut seen_content_only = 0;

    for (label, pairs) in rows {
        let sig = portal_structure_signature(&pairs);
        if shape_changing.contains(&label) {
            assert_ne!(
                base_sig, sig,
                "{label:?} changes which portal pairs are visible, so the structural \
                 signature must move and force a full rebuild"
            );
        } else {
            seen_content_only += 1;
            assert_eq!(
                base_sig, sig,
                "{label:?} changes content only, so the structural signature must hold \
                 still and keep the in-place mutator arm reachable"
            );
        }
    }
    assert_eq!(
        seen_content_only, content_only_count,
        "every corpus row must be classified — an unclassified row is a dimension this \
         test silently stops covering"
    );
}

/// Two visible portal pairs of the same count, laid out
/// identically, under different `EdgeKey`s: the rendered tree
/// cannot tell them apart, and the structural signature must.
///
/// The stake here is sharper than glyphs. `PortalHitIndex` is
/// built from the same pair slice and indexed *positionally* by
/// pair channel, so a pair sequence whose identities changed under
/// a fixed count, with the signature holding still, would leave the
/// previously stamped index in place — and a click on the marker
/// would resolve to the wrong edge, silently.
///
/// Failing input: drop `identity` from
/// `portal_structure_signature`.
#[test]
fn test_portal_structure_signature_moves_when_identities_change_at_a_fixed_count() {
    let base_map = base_portal_map();
    let mut renamed_map = base_portal_map();
    // Same two endpoints, same geometry, a different edge type —
    // and `EdgeKey` is `(from_id, to_id, edge_type)`, so this is a
    // different portal pair carrying identical markers.
    renamed_map.edges[0].edge_type = "parent_child".into();

    let base = portal_pairs(&base_map);
    let renamed = portal_pairs(&renamed_map);
    assert_eq!(base.len(), renamed.len(), "the visible-pair count must not move");
    assert!(
        trees_draw_identically(
            &build_portal_tree_from_pairs(&base).tree,
            &build_portal_tree_from_pairs(&renamed).tree
        ),
        "the two maps must render identically, or this test is measuring a marker change \
         rather than an identity change"
    );
    assert_ne!(
        portal_structure_signature(&base),
        portal_structure_signature(&renamed),
        "the visible pair identities changed under a fixed count, so the structural \
         signature must move — otherwise the stamped hit index keeps naming the old edge \
         for clicks on the new one"
    );
}

/// Neither builder writes a hit box, which is what lets both
/// content signatures leave `GlyphArea::hitbox` out of the hash.
///
/// The exclusion is otherwise unchecked: `GlyphArea::hash_content`
/// skips the field to stay consistent with `PartialEq` and `Hash`,
/// which both ignore it as scene-builder output. Should either
/// builder start stamping click extents, a signature match would
/// skip re-stamping them and clicks would resolve against the
/// previous frame's rectangles — so the precondition is asserted
/// rather than assumed. Failing input: one
/// `area.hitbox_as_mut().add(...)` anywhere in `border.rs` or
/// `portal.rs`.
#[test]
fn test_border_and_portal_areas_carry_no_hitbox() {
    let border_tree = build_border_tree_from_nodes(&border_nodes(&base_border_map(), &HashMap::new(), None));
    let portal_tree = build_portal_tree_from_pairs(&portal_pairs(&base_portal_map())).tree;

    for (role, tree) in [("border", &border_tree), ("portal", &portal_tree)] {
        let mut areas = 0;
        for id in tree.root.descendants(&tree.arena) {
            if let Some(area) = tree.arena.get(id).unwrap().get().glyph_area() {
                areas += 1;
                assert!(
                    area.hitbox().rectangles.is_empty(),
                    "{role} builder stamped a hit box; the content signature does not hash \
                     one, so a skipped update would serve stale click extents"
                );
            }
        }
        assert!(
            areas > 0,
            "{role} fixture emitted no glyph areas, so this test checked nothing"
        );
    }
}
