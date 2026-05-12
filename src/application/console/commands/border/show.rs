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
use baumhard::mindmap::model::MindNode;

use crate::application::console::parser::Args;
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
/// see why their border colour doesn't match —calls
/// this out as a UX bug bake-in.
pub fn execute_border_show(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let id = match first_selected_node_id(&eff.document.selection) {
        Ok(id) => id,
        Err(msg) => return ExecResult::err(msg),
    };
    let side_filter = args
        .kvs()
        .find(|(k, _)| *k == "side")
        .map(|(_, v)| v.to_ascii_lowercase());
    if let Some(ref s) = side_filter {
        if !matches!(s.as_str(), "top" | "bottom" | "left" | "right" | "all") {
            return ExecResult::err(format!(
                "border show: side='{}' unknown; pick top | bottom | left | right | all",
                s
            ));
        }
    }
    let verbose = args
        .positionals()
        .skip(1)
        .any(|p| p.eq_ignore_ascii_case("verbose"));
    let node = match eff.document.mindmap.nodes.get(&id) {
        Some(n) => n,
        None => return ExecResult::err(format!("border: node '{}' not found", id)),
    };
    let mut lines = format_border_readout(
        node,
        eff.document.mindmap.canvas.default_border.as_ref(),
        &eff.document.mindmap.palettes,
        side_filter.as_deref(),
        verbose,
    );
    // API/UX M4: when the selection covers more than one node,
    // the readout silently rolls up to the first one. Prepend a
    // single-line note so the user knows there are siblings —
    // mirrors the `font show` Multi-rollup posture.
    if let Some(extra) = multi_rollup_count(&eff.document.selection) {
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

fn format_border_readout(
    node: &MindNode,
    canvas_default: Option<&baumhard::mindmap::model::GlyphBorderConfig>,
    palettes: &std::collections::HashMap<String, baumhard::mindmap::model::Palette>,
    side_filter: Option<&str>,
    verbose: bool,
) -> Vec<OutputLine> {
    let style = resolve_border_style(
        node.style.border.as_ref(),
        canvas_default,
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
        //surface both color surfaces so the user
        // can see why their border colour doesn't match what they
        // expected. `style.frame_color` is set via `color border=`;
        // `style.border.color` is set via `border color`. Different
        // verbs, different fields, identical-looking authoring.
        lines.push(OutputLine::plain("color (cascade):".to_string()));
        lines.push(OutputLine::plain(format!(
            "  style.frame_color    = {}          # set via `color border=`",
            node.style.frame_color
        )));
        let per_node_color = node.style.border.as_ref().and_then(|c| c.color.as_deref());
        let cascade_target = per_node_color.unwrap_or(node.style.frame_color.as_str());
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
