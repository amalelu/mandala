// SPDX-License-Identifier: MPL-2.0

//! Section-frame tree-builder invariants: per-section Void
//! parents, the eight runs `border_run_specs` lays out, focus
//! style swap, and content-signature coverage for the
//! rebuild-or-skip dispatch.
//!
//! Pins the contract that section frames flow through the same
//! `BorderStyle` machinery node borders use: any preset, any
//! `SidePattern`, any per-corner glyph, any palette cycle a
//! creative-toolkit author can configure ends up in the section
//! frame's GlyphArea text without a separate code path.

use crate::mindmap::border::{resolve_border_style, BorderStyle};
use crate::mindmap::model::{CustomBorderGlyphs, GlyphBorderConfig};
use crate::mindmap::tree_builder::SectionFrameElement;
use crate::mindmap::tree_builder::{build_section_frame_tree, section_frame_content_signature};

/// 10pt is the production floor's font size (private const
/// `SECTION_FRAME_FLOOR_FONT_SIZE_PT` in `border.rs`); this
/// fixture mirrors that value so the test's glyph-layout
/// assertions match what users see, but the value isn't
/// load-bearing — every test below would still pass at any
/// reasonable size.
fn floor_style(focused: bool) -> BorderStyle {
    let cfg = GlyphBorderConfig {
        preset: if focused { "heavy".into() } else { "light".into() },
        font: None,
        font_size_pt: 10.0,
        color: None,
        glyphs: None,
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    };
    resolve_border_style(Some(&cfg), None, None, "#00E5FF")
}

fn frame(node_id: &str, idx: usize, focused: bool) -> SectionFrameElement {
    SectionFrameElement {
        node_id: node_id.to_string(),
        section_idx: idx,
        position: (100.0, 200.0 + idx as f32 * 30.0),
        size: (300.0, 30.0),
        border_style: floor_style(focused),
        palette_cycle: Vec::new(),
        focused,
    }
}

#[test]
fn test_section_frame_tree_empty_input_yields_empty_tree() {
    let tree = build_section_frame_tree(&[]);
    // Only the void root — no children.
    assert_eq!(tree.root.children(&tree.arena).count(), 0);
}

#[test]
fn test_section_frame_tree_one_void_parent_per_frame() {
    let frames = [frame("n", 0, false), frame("n", 1, false), frame("n", 2, false)];
    let tree = build_section_frame_tree(&frames);
    let parents: Vec<_> = tree.root.children(&tree.arena).collect();
    assert_eq!(parents.len(), 3, "one Void parent per frame");
}

#[test]
fn test_section_frame_tree_eight_runs_per_frame() {
    let frames = [frame("n", 0, false)];
    let tree = build_section_frame_tree(&frames);
    let parent = tree.root.children(&tree.arena).next().expect("one parent");
    let runs: Vec<_> = parent.children(&tree.arena).collect();
    // Plan revision 4: 4 fill rails + 4 corners = 8 runs.
    assert_eq!(runs.len(), 8, "4 rails + 4 corners = 8 runs");
}

#[test]
fn test_section_frame_tree_focused_uses_heavy_preset_top_corner() {
    let frames = [frame("n", 0, true)];
    let tree = build_section_frame_tree(&frames);
    let parent = tree.root.children(&tree.arena).next().expect("one parent");
    // Plan revision 4: TL corner is its own spec at index 4
    // (channels 5-8 are corners). Heavy preset uses ┏ (U+250F).
    let tl_corner_id = parent.children(&tree.arena).nth(4).expect("TL corner run");
    let area = tree
        .arena
        .get(tl_corner_id)
        .unwrap()
        .get()
        .glyph_area()
        .expect("GlyphArea");
    assert_eq!(
        area.text, "\u{250F}",
        "focused frame's heavy preset TL must be ┏, got {:?}",
        area.text
    );
}

#[test]
fn test_section_frame_tree_unfocused_uses_light_preset_top_corner() {
    let frames = [frame("n", 0, false)];
    let tree = build_section_frame_tree(&frames);
    let parent = tree.root.children(&tree.arena).next().expect("one parent");
    // Plan revision 4: TL corner at index 4 (channel 5).
    let tl_corner_id = parent.children(&tree.arena).nth(4).expect("TL corner run");
    let area = tree
        .arena
        .get(tl_corner_id)
        .unwrap()
        .get()
        .glyph_area()
        .expect("GlyphArea");
    assert_eq!(
        area.text, "\u{250C}",
        "unfocused frame's light preset TL must be ┌, got {:?}",
        area.text
    );
}

/// Custom-preset frame with author-supplied side / corner glyphs
/// renders the author's chars, **not** the preset defaults. Pins
/// the creative-toolkit contract: any glyph the user types ends
/// up on screen.
#[test]
fn test_section_frame_tree_custom_preset_renders_author_glyphs() {
    let cfg = GlyphBorderConfig {
        preset: "custom".into(),
        font: None,
        font_size_pt: 10.0,
        color: None,
        glyphs: Some(CustomBorderGlyphs {
            top: "A".into(),
            bottom: "B".into(),
            left: "L".into(),
            right: "R".into(),
            top_left: "+".into(),
            top_right: "*".into(),
            bottom_left: "*".into(),
            bottom_right: "+".into(),
        }),
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    };
    let style = resolve_border_style(Some(&cfg), None, None, "#00E5FF");
    let elem = SectionFrameElement {
        node_id: "n".into(),
        section_idx: 0,
        position: (100.0, 200.0),
        size: (300.0, 30.0),
        border_style: style,
        palette_cycle: Vec::new(),
        focused: false,
    };
    let tree = build_section_frame_tree(&[elem]);
    let parent = tree.root.children(&tree.arena).next().expect("one parent");
    // Plan revision 4: top fill rail (index 0, channel 1) carries
    // ONLY the fill `A` glyph — corners are separate specs.
    let top_run_id = parent.children(&tree.arena).next().expect("top run");
    let area = tree
        .arena
        .get(top_run_id)
        .unwrap()
        .get()
        .glyph_area()
        .expect("GlyphArea");
    assert!(
        !area.text.is_empty() && area.text.chars().all(|c| c == 'A'),
        "custom-preset top fill must contain only author's 'A' (no corners baked in); got {:?}",
        area.text
    );
    // TL corner spec (index 4, channel 5) = author's '+'.
    let tl_id = parent.children(&tree.arena).nth(4).expect("TL corner");
    let tl_area = tree
        .arena
        .get(tl_id)
        .unwrap()
        .get()
        .glyph_area()
        .expect("GlyphArea");
    assert_eq!(tl_area.text, "+", "TL corner text");
    // TR corner spec (index 5, channel 6) = author's '*'.
    let tr_id = parent.children(&tree.arena).nth(5).expect("TR corner");
    let tr_area = tree
        .arena
        .get(tr_id)
        .unwrap()
        .get()
        .glyph_area()
        .expect("GlyphArea");
    assert_eq!(tr_area.text, "*", "TR corner text");
    // The fill between corners uses the author's 'A' glyph.
    assert!(
        area.text.contains('A'),
        "custom-preset top fill must include 'A', got {:?}",
        area.text
    );
}

/// Section frame side patterns flow through `border_run_specs`,
/// which understands `SidePattern::PrefixFillSuffix`. Pin that a
/// `top="###(*)###"` pattern renders with the prefix + repeating
/// fill + suffix structure.
#[test]
fn test_section_frame_tree_supports_prefix_fill_suffix_pattern() {
    let cfg = GlyphBorderConfig {
        preset: "custom".into(),
        font: None,
        font_size_pt: 10.0,
        color: None,
        glyphs: Some(CustomBorderGlyphs {
            top: "###(*)###".into(),
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
    };
    let style = resolve_border_style(Some(&cfg), None, None, "#00E5FF");
    let elem = SectionFrameElement {
        node_id: "n".into(),
        section_idx: 0,
        position: (0.0, 0.0),
        size: (400.0, 30.0),
        border_style: style,
        palette_cycle: Vec::new(),
        focused: false,
    };
    let tree = build_section_frame_tree(&[elem]);
    let parent = tree.root.children(&tree.arena).next().expect("one parent");
    let top_run_id = parent.children(&tree.arena).next().expect("top run");
    let area = tree
        .arena
        .get(top_run_id)
        .unwrap()
        .get()
        .glyph_area()
        .expect("GlyphArea");
    // The top text should contain at least one `*` (the fill)
    // between two `###` static segments.
    assert!(
        area.text.contains("###"),
        "top must contain prefix '###', got {:?}",
        area.text
    );
    assert!(
        area.text.contains('*'),
        "top must contain fill '*', got {:?}",
        area.text
    );
}

#[test]
fn test_section_frame_content_signature_includes_focus_flag() {
    let unfocused = [frame("n", 0, false)];
    let focused = [frame("n", 0, true)];
    let s_unfocused = section_frame_content_signature(&unfocused);
    let s_focused = section_frame_content_signature(&focused);
    assert_ne!(
        s_unfocused, s_focused,
        "focus toggle must change the content signature so the dispatch rebuilds the glyphs"
    );
}

#[test]
fn test_section_frame_content_signature_stable_for_unchanged_inputs() {
    let frames = [frame("n", 0, false), frame("n", 1, true)];
    assert_eq!(
        section_frame_content_signature(&frames),
        section_frame_content_signature(&frames),
        "identity is a pure function of inputs"
    );
}

/// A color edit on a section frame must move the content
/// signature so the dispatch rebuilds the glyph runs with the new
/// tint. Pin so a future regression that stops hashing the
/// resolved `BorderStyle.color` fails this test.
#[test]
fn test_section_frame_content_signature_changes_on_color_change() {
    let mut a = frame("n", 0, false);
    let mut b = frame("n", 0, false);
    a.border_style.color = "#ff0000".into();
    b.border_style.color = "#00ff00".into();
    assert_ne!(
        section_frame_content_signature(&[a]),
        section_frame_content_signature(&[b]),
        "color change must change the signature"
    );
}

/// A preset swap (light → heavy) changes the rendered glyphs on
/// each side, so the signature must move. Without this the
/// dispatch would skip a rebuild and leave stale ┌─┐ corners on
/// a frame that should be ┏━┓.
#[test]
fn test_section_frame_content_signature_changes_on_preset_change() {
    let a = SectionFrameElement {
        border_style: floor_style(false), // light
        ..frame("n", 0, false)
    };
    let b = SectionFrameElement {
        border_style: floor_style(true), // heavy
        ..frame("n", 0, false)
    };
    assert_ne!(
        section_frame_content_signature(&[a]),
        section_frame_content_signature(&[b]),
        "preset change must change the signature (different glyphs on every side)"
    );
}

/// A custom-pattern swap (`top="A"` → `top="B"`) must change the
/// signature. The identity captures rendered side text via
/// `border_run_specs`, so any author edit to a `SidePattern` lands
/// in the dispatch path.
#[test]
fn test_section_frame_content_signature_changes_on_pattern_change() {
    let cfg_a = GlyphBorderConfig {
        preset: "custom".into(),
        font: None,
        font_size_pt: 10.0,
        color: None,
        glyphs: Some(CustomBorderGlyphs {
            top: "A".into(),
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
    };
    let mut cfg_b = cfg_a.clone();
    if let Some(g) = cfg_b.glyphs.as_mut() {
        g.top = "B".into();
    }
    let style_a = resolve_border_style(Some(&cfg_a), None, None, "#00E5FF");
    let style_b = resolve_border_style(Some(&cfg_b), None, None, "#00E5FF");
    let a = SectionFrameElement {
        border_style: style_a,
        ..frame("n", 0, false)
    };
    let b = SectionFrameElement {
        border_style: style_b,
        ..frame("n", 0, false)
    };
    assert_ne!(
        section_frame_content_signature(&[a]),
        section_frame_content_signature(&[b]),
        "side-pattern change must change the signature"
    );
}

/// `font_size_pt` change must move the signature even when the
/// resulting `cluster_count` happens to land on the same integer
/// (e.g. 14.0 → 14.5 on a 100-px wide section both compute
/// `((100/(14.0*0.6))+2).ceil() = 14`). Pre-fix the signature hashed
/// the rendered text only; this test pins the new inputs-hash so a
/// future regression can't reintroduce the rendered-output shortcut.
#[test]
fn test_section_frame_content_signature_changes_on_font_size_change() {
    let mut a = frame("n", 0, false);
    let mut b = frame("n", 0, false);
    a.border_style.font_size_pt = 14.0;
    b.border_style.font_size_pt = 14.5;
    assert_ne!(
        section_frame_content_signature(&[a]),
        section_frame_content_signature(&[b]),
        "font_size_pt change must change the signature"
    );
}

/// `font_name` change must move the signature. The renderer
/// pipeline doesn't yet thread `BorderStyle.font_name` into the
/// emitted `GlyphArea`, but the resolver populates it and the
/// signature should track every input the model exposes — when
/// the renderer wires it up, the dispatch already triggers
/// rebuilds on the right shape.
#[test]
fn test_section_frame_content_signature_changes_on_font_name_change() {
    let mut a = frame("n", 0, false);
    let mut b = frame("n", 0, false);
    a.border_style.font_name = None;
    b.border_style.font_name = Some("Liberation Mono".into());
    assert_ne!(
        section_frame_content_signature(&[a]),
        section_frame_content_signature(&[b]),
        "font_name change must change the signature"
    );
}

/// Position change with same rendered text must move the
/// signature. Pre-fix the signature didn't include position, so
/// dragging the active node while in NodeEdit (which calls
/// `update_section_frame_tree` once the drag fix lands) would hit
/// `InPlaceMutator` and the registered tree would render at the
/// pre-drag origin.
#[test]
fn test_section_frame_content_signature_changes_on_position_change() {
    let mut a = frame("n", 0, false);
    let mut b = frame("n", 0, false);
    a.position = (100.0, 200.0);
    b.position = (110.0, 200.0);
    assert_ne!(
        section_frame_content_signature(&[a]),
        section_frame_content_signature(&[b]),
        "position change must change the signature"
    );
}

/// Bounds (size) change with same text must move the signature
/// for the same reason position changes do.
#[test]
fn test_section_frame_content_signature_changes_on_size_change() {
    let mut a = frame("n", 0, false);
    let mut b = frame("n", 0, false);
    a.size = (300.0, 30.0);
    b.size = (320.0, 30.0);
    assert_ne!(
        section_frame_content_signature(&[a]),
        section_frame_content_signature(&[b]),
        "size change must change the signature"
    );
}

/// Palette-cycle list change must move the signature so an
/// authored palette edit (`section frame palette=…`) triggers a
/// rebuild. The cycle is the resolved per-glyph color sequence —
/// not the palette name — so a palette whose hex strings update
/// in place is also caught.
#[test]
fn test_section_frame_content_signature_changes_on_palette_cycle_change() {
    let mut a = frame("n", 0, false);
    let mut b = frame("n", 0, false);
    a.palette_cycle = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]];
    b.palette_cycle = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]];
    assert_ne!(
        section_frame_content_signature(&[a]),
        section_frame_content_signature(&[b]),
        "palette_cycle change must change the signature"
    );
}
