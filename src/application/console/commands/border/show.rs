// SPDX-License-Identifier: MPL-2.0

//! `border show` — multi-line readout of the resolved border
//! config for the current selection.
//!
//! Each side row renders in the resolved border font so the user
//! previews the chosen face inline. Operates on the *first*
//! selected node (multi-selection rolls up to one summary; the
//! per-node config may differ but a single readout is cleaner
//! than four-up).

use baumhard::mindmap::border::{resolve_border_style, BORDER_APPROX_CHAR_WIDTH_FRAC};
use baumhard::mindmap::border_pattern::SidePattern;
use baumhard::mindmap::model::{MindMap, MindNode};

use crate::application::console::parser::Args;
use crate::application::console::spec::{kvs, Descent};
use crate::application::console::{ConsoleEffects, ExecResult, OutputLine};
use crate::application::document::SelectionState;

/// `border show [side=<top|bottom|left|right|all>] [verbose]`
/// —/ §5.3.
///
/// `side=` filters to one of the four sides plus the matching
/// corners — useful when the user only wants to see what a
/// single side looks like (the default 11-line readout is
/// scrollback-heavy).
///
/// `verbose` is a bare positional that surfaces the dual color
/// surface (`style.frame_color` set via `color border=…` vs
/// `style.border.color` set via `border color`) so the user can
/// see why their border color doesn't match —calls
/// this out as a UX bug bake-in.
pub fn execute_border_show(descent: &Descent, args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // `side=` is a key of the *readout*, not of the border
    // configuration, so the grammar declares it on this subverb
    // alone: the composed kv form does not read it and the popup at
    // that slot does not offer it.
    let pairs = match kvs::read_strict(descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    let id = match first_selected_node_id(&eff.document().selection) {
        Ok(id) => id,
        Err(msg) => return ExecResult::err(msg),
    };
    let side_filter = kvs::value(&pairs, "side").map(|v| v.to_ascii_lowercase());
    if let Some(ref s) = side_filter {
        if !matches!(s.as_str(), "top" | "bottom" | "left" | "right" | "all") {
            return ExecResult::err(format!(
                "border show: side='{}' unknown; pick top | bottom | left | right | all",
                s
            ));
        }
    }
    let verbose = descent
        .slot_value(args)
        .get(0)
        .is_some_and(|p| p.eq_ignore_ascii_case("verbose"));
    let node = match eff.document().mindmap.nodes.get(&id) {
        Some(n) => n,
        None => return ExecResult::err(format!("border: node '{}' not found", id)),
    };
    let mut lines = format_border_readout(&eff.document().mindmap, node, side_filter.as_deref(), verbose);
    // API/UX M4: when the selection covers more than one node,
    // the readout silently rolls up to the first one. Prepend a
    // single-line note so the user knows there are siblings —
    // mirrors the `font show` Multi-rollup posture.
    if let Some(extra) = multi_rollup_count(&eff.document().selection) {
        lines.insert(
            0,
            crate::application::console::OutputLine::plain(format!(
                "note: showing first of {} selected nodes; per-node configs may differ",
                extra
            )),
        );
    }
    ExecResult::Lines(lines)
}

/// `Some(n)` when the selection covers more than one node-shape
/// target (so `border show` rolled up); `None` otherwise.
/// `Section` / `SectionRange` collapse to one owning node so
/// they don't trigger the rollup hint.
fn multi_rollup_count(sel: &SelectionState) -> Option<usize> {
    match sel {
        SelectionState::Multi(ids) if ids.len() > 1 => Some(ids.len()),
        SelectionState::MultiSection(secs) if secs.len() > 1 => Some(secs.len()),
        _ => None,
    }
}

/// Resolve the first node id `border show` should report on.
/// Section / SectionRange / MultiSection selections collapse
/// to the section's owning node — same posture every other
/// border subverb takes via `nodes_in_selection` (Section
/// borders attach to the node, not the section). Pre-fix
/// `border show` rejected those variants despite the verb's
/// `node_or_section_selected` predicate advertising them.
fn first_selected_node_id(sel: &SelectionState) -> Result<String, String> {
    match sel {
        SelectionState::Single(id) => Ok(id.clone()),
        SelectionState::Multi(ids) => ids
            .first()
            .cloned()
            .ok_or_else(|| "border: empty selection".to_string()),
        SelectionState::Section(s) => Ok(s.node_id.clone()),
        SelectionState::SectionRange { sel: s, .. } => Ok(s.node_id.clone()),
        SelectionState::MultiSection(secs) => secs
            .first()
            .map(|s| s.node_id.clone())
            .ok_or_else(|| "border: empty selection".to_string()),
        SelectionState::None => Err("border: no selection".to_string()),
        _ => Err("border: not applicable to this selection".to_string()),
    }
}

/// Which tier of the palette cascade the node's frame color came
/// from, phrased for a console readout. The whole point of
/// `border show verbose` is telling the user *where* a color they
/// did not expect is coming from, and "the palette" is now one of
/// the answers.
fn frame_color_source(map: &MindMap, node: &MindNode) -> String {
    let Some(schema) = node.color_schema.as_ref() else {
        return "style.frame_color".to_string();
    };
    // Ask the reader whether a themed tier applies at all, rather
    // than re-deciding here. `node_frame_theme_tier` skips an
    // **empty** frame in both slots — an empty string is a hole, not
    // a color — so `overrides.frame == Some("")` and a resolved group
    // whose `frame` is empty both fall through to `style`. Deciding
    // it twice is how this readout came to name a tier the renderer
    // had already skipped, which is the one thing this verb exists
    // not to do.
    if map.node_frame_theme_tier(node).is_none() {
        return match map.resolve_theme_colors(node) {
            Some(_) => format!(
                "style.frame_color (palette `{}` sets no frame at level {})",
                schema.palette, schema.level
            ),
            None => format!(
                "style.frame_color (palette `{}` does not resolve)",
                schema.palette
            ),
        };
    }
    if schema.overrides.frame.as_deref().is_some_and(|f| !f.is_empty()) {
        return format!(
            "color_schema.overrides.frame (excepting palette `{}`)",
            schema.palette
        );
    }
    format!("palette `{}` at level {}", schema.palette, schema.level)
}

fn format_border_readout(
    map: &MindMap,
    node: &MindNode,
    side_filter: Option<&str>,
    verbose: bool,
) -> Vec<OutputLine> {
    let canvas_default = map.canvas.default_border.as_ref();
    let palettes = &map.palettes;
    // The **effective** frame color, through the same cascade the
    // render path uses (`tree_builder/border.rs`). A readout that
    // reached for `node.style.frame_color` here would print
    // `#222222` while the glyphs paint the node's palette frame —
    // which is precisely the confusion this verb exists to clear
    // up.
    let style = resolve_border_style(
        node.style.border.as_ref(),
        canvas_default,
        map.node_frame_theme_tier(node),
        &node.style.frame_color,
    );
    let approx_char_width = style.font_size_pt * BORDER_APPROX_CHAR_WIDTH_FRAC;
    let size = node.size_vec2();
    let char_count = ((size.x / approx_char_width) + 2.0).ceil().max(3.0) as usize;
    let row_count = (size.y / style.font_size_pt).round().max(1.0) as usize;

    let preset_name = node
        .style
        .border
        .as_ref()
        .map(|c| c.preset.as_str())
        .unwrap_or("(default)");
    let visible = node.style.show_frame;

    let palette_summary = match style.color_palette.as_ref() {
        Some(name) => match palettes.get(name) {
            Some(p) => format!(
                "{} (cycling '{}' across {} groups)",
                name,
                style.palette_field.as_str(),
                p.groups.len()
            ),
            None => format!("{} (not found in map)", name),
        },
        None => "(none)".to_string(),
    };

    let face = style.font_name.clone();
    let mut lines: Vec<OutputLine> = Vec::with_capacity(12);
    //inline action hints: each row's right-hand side
    // surfaces the verb that flips that field, so the readout
    // doubles as a discoverability surface — users learn the
    // verb shape from the show output.
    lines.push(OutputLine::plain(format!(
        "visible: {:<3}  (toggle: `border toggle`)",
        if visible { "on" } else { "off" }
    )));
    lines.push(OutputLine::plain(format!(
        "preset:  {:<8}  (cycle: `border preset cycle`)",
        preset_name
    )));
    let face_str = face.as_deref().unwrap_or("(default)");
    let font_override = node
        .style
        .border
        .as_ref()
        .and_then(|c| c.font.as_deref())
        .map(|_| "per-node override")
        .or_else(|| {
            canvas_default
                .and_then(|c| c.font.as_deref())
                .map(|_| "canvas default")
        })
        .unwrap_or("hardcoded floor");
    lines.push(OutputLine::plain(format!(
        "font:    {} ({} pt)  (override: `border font <family>`, source: {})",
        face_str, style.font_size_pt, font_override
    )));
    if verbose {
        // Surface every color surface the border sits on, so the
        // user can see why their border color doesn't match what
        // they expected. Three of them now: the node's *effective*
        // frame color and where it came from, the raw
        // `style.frame_color` the palette may be shadowing, and
        // `style.border.color`, which outranks both. Printing only
        // the raw field — as this did — reported `#222222` on a
        // node whose glyphs paint its palette's `#a20000`, which
        // is exactly the confusion the verb exists to clear up.
        lines.push(OutputLine::plain("color (cascade):".to_string()));
        let frame_color = map.node_frame_color(node);
        lines.push(OutputLine::plain(format!(
            "  node frame (effective) = {}          # source: {}",
            frame_color,
            frame_color_source(map, node)
        )));
        lines.push(OutputLine::plain(format!(
            "  style.frame_color    = {}          # the unthemed tier, set via `color border=`",
            node.style.frame_color
        )));
        let per_node_color = node.style.border.as_ref().and_then(|c| c.color.as_deref());
        let cascade_target = per_node_color.unwrap_or(frame_color);
        let per_node_label = per_node_color.unwrap_or("(unset)");
        lines.push(OutputLine::plain(format!(
            "  style.border.color   = {} [\u{2192} {}]   # set via `border color`",
            per_node_label, cascade_target
        )));
    } else {
        lines.push(OutputLine::plain(format!("color:   {}", style.color)));
    }
    lines.push(OutputLine::plain(format!("palette: {}", palette_summary)));
    // Padding cascades per-node → canvas-default → 4px hardcoded
    // floor. Always print the resolved value so the readout is
    // useful even for nodes with no per-node override (the canvas
    // default may still set a non-default padding).
    let resolved_padding = node
        .style
        .border
        .as_ref()
        .map(|c| c.padding)
        .or_else(|| canvas_default.map(|c| c.padding))
        .unwrap_or(4.0);
    lines.push(OutputLine::plain(format!("padding: {} px", resolved_padding)));
    lines.push(OutputLine::plain(format!(
        "size:    {}×{} px ({} cluster cols, {} rows)",
        node.size.width as i64, node.size.height as i64, char_count, row_count
    )));

    let side_face = face.clone();
    let want = |s: &str| match side_filter {
        None => true,
        Some("all") => true,
        Some(f) => f == s,
    };
    if want("top") {
        lines.push(side_line(
            "top:    ",
            &style.side_patterns.top,
            char_count,
            &side_face,
        ));
    }
    if want("bottom") {
        lines.push(side_line(
            "bottom: ",
            &style.side_patterns.bottom,
            char_count,
            &side_face,
        ));
    }
    if want("left") {
        lines.push(side_line(
            "left:   ",
            &style.side_patterns.left,
            row_count,
            &side_face,
        ));
    }
    if want("right") {
        lines.push(side_line(
            "right:  ",
            &style.side_patterns.right,
            row_count,
            &side_face,
        ));
    }
    // Corners are pruned alongside any side filter — `top`
    // shouldn't include bl / br corners.
    if side_filter.is_none() || side_filter == Some("all") {
        lines.push(corner_line(&style, &face));
    }
    lines
}

fn side_line(label: &str, pattern: &SidePattern, width: usize, face: &Option<String>) -> OutputLine {
    let rendered = pattern.render(width).text;
    let text = format!("{}{}", label, rendered);
    match face {
        Some(family) => OutputLine::in_font(text, family),
        None => OutputLine::plain(text),
    }
}

fn corner_line(style: &baumhard::mindmap::border::BorderStyle, face: &Option<String>) -> OutputLine {
    let text = format!(
        "corners: tl={}  tr={}  bl={}  br={}",
        style.corners.top_left,
        style.corners.top_right,
        style.corners.bottom_left,
        style.corners.bottom_right
    );
    match face {
        Some(family) => OutputLine::in_font(text, family),
        None => OutputLine::plain(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::{first_testament_node_id, load_test_doc};
    use baumhard::mindmap::model::MindMap;

    fn readout_blob(map: &MindMap, node: &MindNode, verbose: bool) -> String {
        format_border_readout(map, node, None, verbose)
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The readout has to agree with the glyphs. Every testament
    /// node is themed, so a `border show` that reached for
    /// `style.frame_color` printed a color the border is not
    /// painted in — the verb's own doc comment says it exists so
    /// the user can see why their border color does not match what
    /// they expected, and it was the thing lying to them.
    #[test]
    fn test_border_show_reports_the_effective_frame_color() {
        // `maps/palette_cascade.mindmap.json` rather than
        // testament: its `style` values are deliberately different
        // from its palette's, where testament's have drifted
        // toward each other and would let a wrong read pass.
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("maps/palette_cascade.mindmap.json");
        let map =
            baumhard::mindmap::loader::load_from_file(&path).expect("the palette-cascade fixture must load");
        let node = map.nodes.get("0").expect("fixture node 0");
        assert!(node.color_schema.is_some(), "fixture node 0 is themed");
        let effective = map.node_frame_color(node).to_string();
        assert_ne!(
            effective, node.style.frame_color,
            "the fixture must keep its tiers apart or this test proves nothing"
        );
        let blob = readout_blob(&map, node, false);
        assert!(
            blob.contains(&format!("color:   {effective}")),
            "readout must print the color the border is painted in: {blob}"
        );
    }

    /// `verbose` names the tier the color came from, which is the
    /// whole reason the flag exists. A themed node reports its
    /// palette; a per-node override reports itself.
    #[test]
    fn test_border_show_verbose_names_the_cascade_tier() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let palette = doc
            .mindmap
            .nodes
            .get(&id)
            .and_then(|n| n.color_schema.as_ref())
            .map(|s| s.palette.clone())
            .expect("testament nodes are themed");
        let node = doc.mindmap.nodes.get(&id).unwrap();
        let blob = readout_blob(&doc.mindmap, node, true);
        assert!(
            blob.contains(&format!("palette `{palette}`")),
            "a themed node must be told its color comes from the palette: {blob}"
        );

        assert!(doc.set_node_border_color(&id, Some("#00ff00")));
        let node = doc.mindmap.nodes.get(&id).unwrap();
        let blob = readout_blob(&doc.mindmap, node, true);
        assert!(
            blob.contains("color_schema.overrides.frame"),
            "a hand-recolored node must be told the override is what won: {blob}"
        );
        assert!(
            blob.contains("#00ff00"),
            "...and the effective color must be the one just set: {blob}"
        );
    }

    /// An **empty** frame is a hole, not a color. The readout has
    /// to name the tier the renderer actually used, and the two
    /// slots fail differently:
    ///
    /// - an empty **override** is skipped and the *group* wins, so
    ///   the answer is the palette — the old code answered
    ///   `overrides.frame` because it only asked `is_some()`;
    /// - an empty **group** with no override leaves no themed tier
    ///   at all, so the answer is `style.frame_color` — the old
    ///   code answered `palette` because it only asked whether
    ///   `resolve_theme_colors` returned `Some`.
    ///
    /// Both came from deciding the tier a second time here instead
    /// of asking the reader, which is the one thing this verb
    /// exists not to do. No setter can write an empty override, so
    /// only a hand-authored file reaches it — which is why nothing
    /// caught it.
    #[test]
    fn test_border_show_verbose_does_not_name_a_tier_the_renderer_skipped() {
        fn empty_the_group(map: &mut MindMap, id: &str) {
            let schema = map
                .nodes
                .get(id)
                .and_then(|n| n.color_schema.as_ref())
                .expect("testament nodes are themed");
            let palette: String = schema.palette.clone();
            let level: usize = schema.level;
            if let Some(entry) = map.palettes.get_mut(&palette) {
                let idx = level.min(entry.groups.len().saturating_sub(1));
                if let Some(group) = entry.groups.get_mut(idx) {
                    group.frame = String::new();
                }
            }
        }

        // An empty override falls through to a live group: the
        // palette is the winning tier, not the override.
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let palette = {
            let node = doc.mindmap.nodes.get_mut(&id).expect("testament node");
            let schema = node.color_schema.as_mut().expect("themed");
            schema.overrides.frame = Some(String::new());
            schema.palette.clone()
        };
        let node = doc.mindmap.nodes.get(&id).expect("testament node");
        assert!(
            doc.mindmap.node_frame_theme_tier(node).is_some(),
            "this case needs a live group frame or it proves nothing"
        );
        let blob = readout_blob(&doc.mindmap, node, true);
        assert!(
            !blob.contains("color_schema.overrides.frame"),
            "an empty override is skipped by the reader and must not be named: {blob}"
        );
        assert!(
            blob.contains(&format!("palette `{palette}`")),
            "the group is what the renderer used, so the readout must say so: {blob}"
        );

        // Empty the group too: now no themed tier survives at all.
        empty_the_group(&mut doc.mindmap, &id);
        let node = doc.mindmap.nodes.get(&id).expect("testament node");
        assert!(
            doc.mindmap.node_frame_theme_tier(node).is_none(),
            "both slots are empty, so there is no themed tier"
        );
        let blob = readout_blob(&doc.mindmap, node, true);
        assert!(
            blob.contains("style.frame_color"),
            "with both slots empty the border comes from style: {blob}"
        );

        // An empty group with no override at all: same answer, and
        // the tier that resolved must still not be claimed.
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        empty_the_group(&mut doc.mindmap, &id);
        let node = doc.mindmap.nodes.get(&id).expect("testament node");
        assert!(
            doc.mindmap.resolve_theme_colors(node).is_some(),
            "the group still resolves"
        );
        assert!(
            doc.mindmap.node_frame_theme_tier(node).is_none(),
            "...but its frame is empty, so the themed tier is a hole"
        );
        let blob = readout_blob(&doc.mindmap, node, true);
        assert!(
            blob.contains("style.frame_color"),
            "a resolved group with an empty frame still falls through to style: {blob}"
        );
    }
}
