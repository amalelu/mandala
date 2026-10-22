// SPDX-License-Identifier: MPL-2.0

//! The palette cascade — the one place that answers "what color is
//! this node, and what color is the edge leaving it?"
//!
//! A node binds itself to a map-level [`Palette`](super::Palette) by
//! carrying a [`ColorSchema`](super::ColorSchema): a palette name, a
//! depth `level`, two miMind-inherited flags, and this node's own
//! exceptions to the group ([`ColorOverrides`]).
//! Resolution walks *override first, palette second, `style` third*.
//!
//! The middle tier is the reason palettes were hoisted out of the
//! nodes during migration: the palette, not the node, is where a
//! *theme* edit lands. The top tier is the reason a per-node edit
//! still works: a theme is inherited and a direct edit is specific,
//! so the direct edit wins — the same rule a [`TextRun`](super::TextRun)
//! color and a `border.color` already follow one level down. It
//! cannot be `style`: `style` is what the palette shadows, and every
//! migrated node carries baked `style` colors that are stale copies
//! of its own theme. `format/palettes.md` is the normative spec;
//! this module is its implementation, and the projection passes
//! under [`crate::mindmap::tree_builder`] read colors only through
//! the four `node_*` / `edge_*` readers below.
//!
//! ## Why the readers return `&str` and not a parsed color
//!
//! Every tier stores authored strings — `#rrggbb`, `#rrggbbaa`, or a
//! `var(--name)` reference the canvas theme map expands later. The
//! cascade's job is to pick *which string*, and
//! [`crate::util::color::resolve_var`] is a separate, later step that
//! every caller already performs. Returning a borrow keeps the whole
//! cascade allocation-free on a per-node, per-frame path (§4's mobile
//! budget).

use super::{ColorGroup, ColorOverrides, MindEdge, MindMap, MindNode};
use crate::util::color::{hex_to_rgba_safe, resolve_var, FloatRgba};

/// One channel of a node's own [`ColorOverrides`], or `None` when
/// the node is unthemed or has no opinion on that channel.
///
/// The tier above the palette group in all four readers below.
/// Reaching it needs no palette lookup and no `resolve_theme_colors`
/// — an override stands even when the schema's palette is missing,
/// because "I painted this node green" does not stop being true when
/// the theme it excepts breaks.
fn channel_override<'a>(
    node: &'a MindNode,
    pick: impl Fn(&'a ColorOverrides) -> Option<&'a str>,
) -> Option<&'a str> {
    pick(&node.color_schema.as_ref()?.overrides)
}

/// `None` for the empty string, `Some` otherwise.
///
/// **Which channels use it is the whole content of the rule.** An
/// empty `background` is a value: the node pass reads it as "no
/// fill, let the canvas show through", so it has to be passed
/// along. An empty `frame`, `text` or `title` is not — none of
/// them has a transparent spelling (a frame is switched off with
/// `show_frame`, and text has no off), so an empty one reaches
/// `hex_to_rgba_safe` and paints opaque black. `title` already
/// fell through; the other two now do too, which is the symmetry
/// `format/schema.md` describes.
///
/// It runs on the [`ColorOverrides`] tier as well as the group,
/// and the reason there is not the group's. A hole in the group
/// would lose the `style` value it was shadowing; a hole in an
/// override is a *node* claiming a color it has not named, which
/// is not a claim the format can honor. So the readers skip it —
/// and, symmetrically, the setters never write one: a per-node
/// write of the empty string to `frame` or `text` clears the slot
/// instead, because storing a value the reader is guaranteed to
/// skip is the "reported success, changed nothing on screen"
/// failure this whole cascade exists to make impossible. A
/// hand-authored file can still carry one, which is why the skip
/// stays.
fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

impl MindMap {
    /// Resolve the [`ColorGroup`] a themed node draws from, or
    /// `None` when the node is unthemed and its `style` colors
    /// stand.
    ///
    /// Four ways to get `None`, each meaning "fall back to
    /// `style`":
    ///
    /// 1. The node carries no `color_schema` at all.
    /// 2. `schema.palette` names a palette this map does not have.
    ///    `maptool verify` reports that as an error; the renderer
    ///    degrades instead of failing the frame
    ///    (`CODE_CONVENTIONS.md` §9).
    /// 3. The palette exists but has no groups — likewise a
    ///    `verify` error.
    /// 4. `starts_at_root` is `false` and the node *is* the schema
    ///    root (`level == 0`). The flag's whole meaning is that
    ///    level 0 belongs to the root's children, so the root
    ///    itself is left transparent and keeps its own `style`.
    ///
    /// Group index is `level` when `starts_at_root`, else
    /// `level - 1`. An index past the last group **clamps to the
    /// last group** rather than wrapping, so a subtree deeper than
    /// its palette degrades to a single color instead of cycling
    /// back to the root's.
    ///
    /// Cost: one `HashMap` lookup and one slice index. No
    /// allocation.
    pub fn resolve_theme_colors<'a>(&'a self, node: &'a MindNode) -> Option<&'a ColorGroup> {
        let schema = node.color_schema.as_ref()?;
        let palette = self.palettes.get(&schema.palette)?;
        let index = if schema.starts_at_root {
            schema.level
        } else {
            schema.level.checked_sub(1)?
        };
        palette.groups.get(index).or_else(|| palette.groups.last())
    }

    /// The authored **fill** color for a node: this node's own
    /// `background` override, else the resolved palette group's
    /// `background`, else `node.style.background_color`.
    ///
    /// The empty string is a meaningful value downstream — the node
    /// pass reads it as "no fill, let the canvas show through" — so
    /// it is passed along rather than treated as a miss. Cost: one
    /// [`Self::resolve_theme_colors`].
    pub fn node_background_color<'a>(&'a self, node: &'a MindNode) -> &'a str {
        if let Some(own) = channel_override(node, |o| o.background.as_deref()) {
            return own;
        }
        match self.resolve_theme_colors(node) {
            Some(group) => group.background.as_str(),
            None => node.style.background_color.as_str(),
        }
    }

    /// The frame color the node supplies **for itself** — its own
    /// `frame` override or its palette group's `frame` — or `None`
    /// when the only answer left is `style.frame_color`.
    ///
    /// Split out from [`Self::node_frame_color`] because the two
    /// halves sit on opposite sides of the *canvas-wide* border
    /// default in `mindmap::border::resolve_border_style`. A
    /// per-node theme is more specific than a map-wide border
    /// color and outranks it; `style.frame_color` is the floor
    /// underneath everything. Collapsing them into one string
    /// would make one map-wide `canvas border color=…` flatten
    /// every palette-derived frame in the document with no
    /// per-node recovery.
    ///
    /// It is also the whole of what a themed node contributes to
    /// the **edges leaving it** —
    /// [`Self::edge_theme_stroke_color`] calls this rather than
    /// reaching for the group's `frame` itself, so the frame
    /// channel is read at one tier by both ladders and a per-node
    /// override cannot move a node's border while leaving its
    /// branch strokes behind.
    ///
    /// Cost: one [`Self::resolve_theme_colors`]. No allocation.
    pub fn node_frame_theme_tier<'a>(&'a self, node: &'a MindNode) -> Option<&'a str> {
        if let Some(own) = channel_override(node, |o| o.frame.as_deref()).and_then(non_empty) {
            return Some(own);
        }
        non_empty(self.resolve_theme_colors(node)?.frame.as_str())
    }

    /// The authored **frame** color for a node: this node's own
    /// `frame` override, else the resolved palette group's `frame`,
    /// else `node.style.frame_color`.
    ///
    /// An **empty** group `frame` is not a color and falls
    /// through to `style.frame_color`, the same way an empty
    /// `title` falls through to `text`. Only `background` passes
    /// an empty string along, where it spells "transparent".
    ///
    /// This is the cascade base the border resolver
    /// (`mindmap::border::resolve_border_style`) sits on top of, so
    /// a per-node `border.color` override still wins over the
    /// theme — an explicit choice beats an inherited one. Callers
    /// that also have to place the *canvas-wide* border default in
    /// that cascade want [`Self::node_frame_theme_tier`] and
    /// `node.style.frame_color` separately; this collapses the two.
    /// Cost: one [`Self::resolve_theme_colors`].
    pub fn node_frame_color<'a>(&'a self, node: &'a MindNode) -> &'a str {
        self.node_frame_theme_tier(node)
            .unwrap_or(node.style.frame_color.as_str())
    }

    /// The authored **text** color for a node: this node's own
    /// `text` override, else the resolved palette group's `text`,
    /// else `node.style.text_color`.
    ///
    /// An **empty** group `text` is not a color; it falls through
    /// to `style.text_color`, exactly as an empty `title` falls
    /// through to `text`. Only `background` passes an empty string
    /// along, because there it spells "transparent".
    ///
    /// This is the section-level default, not a per-grapheme one. A
    /// [`TextRun`](super::TextRun) carrying a non-empty `color`
    /// keeps it: a run is a deliberate per-slice override and the
    /// theme must not repaint it, exactly as an inline style beats
    /// an inherited one. The theme reaches text through the runs
    /// that *declined* to name a color and through sections that
    /// carry no runs at all. Cost: one
    /// [`Self::resolve_theme_colors`].
    pub fn node_text_color<'a>(&'a self, node: &'a MindNode) -> &'a str {
        if let Some(own) = channel_override(node, |o| o.text.as_deref()).and_then(non_empty) {
            return own;
        }
        self.resolve_theme_colors(node)
            .and_then(|group| non_empty(group.text.as_str()))
            .unwrap_or(node.style.text_color.as_str())
    }

    /// The authored **title** color for a node — the first-line
    /// stand-in for [`Self::node_text_color`].
    ///
    /// Only the palette and this node's own overrides carry this
    /// channel; [`NodeStyle`] has no title field, so an unthemed
    /// node — or a themed one whose group leaves `title` empty and
    /// which overrides nothing — returns the node's text color and
    /// the first line is not distinguished.
    ///
    /// [`NodeStyle`]: super::NodeStyle
    ///
    /// Cost: one [`Self::resolve_theme_colors`].
    pub fn node_title_color<'a>(&'a self, node: &'a MindNode) -> &'a str {
        if let Some(own) = channel_override(node, |o| o.title.as_deref()).and_then(non_empty) {
            return own;
        }
        self.resolve_theme_colors(node)
            .and_then(|group| non_empty(group.title.as_str()))
            .unwrap_or_else(|| self.node_text_color(node))
    }

    /// [`Self::node_text_color`] carried the whole way to pixels —
    /// through the canvas theme variables and through
    /// [`crate::util::color::hex_to_rgba_safe`], with opaque black
    /// as the floor for an unparseable string.
    ///
    /// This is the value a section's glyphs are actually painted
    /// in when nothing more specific claims them, so it is what
    /// both the forward projection and any reverse comparison must
    /// use. Keeping it here rather than at the three call sites is
    /// what stops the projection and the comparison from drifting:
    /// a reverse gate that resolved the string differently from
    /// the forward pass would read every themed section as
    /// divergent.
    ///
    /// Cost: one [`Self::node_text_color`] plus one hex parse. No
    /// allocation — `resolve_var` borrows when there is nothing to
    /// substitute.
    pub fn node_text_rgba(&self, node: &MindNode) -> FloatRgba {
        let resolved = resolve_var(self.node_text_color(node), &self.canvas.theme_variables);
        hex_to_rgba_safe(resolved, [0.0, 0.0, 0.0, 1.0])
    }

    /// The palette stroke color an edge inherits from its **source**
    /// node, or `None` when no theme tier applies.
    ///
    /// This is the tier [`MindEdge::body_color`] takes as its
    /// `themed` argument; resolve it here rather than at the call
    /// site so every consumer of the edge color cascade agrees on
    /// what "the theme" means for an edge.
    ///
    /// Two conditions gate it: the source node must resolve to a
    /// palette group at all, and that node's
    /// `color_schema.connections_colored` must be `true` — the flag
    /// is per-schema precisely so a themed subtree can keep its
    /// node fills while leaving its connections alone.
    ///
    /// **The source node, not the target.** An edge is drawn in its
    /// parent's branch color, which is what the miMind-derived
    /// corpus shows. Counted over the 248 `parent_child` edges in
    /// `maps/testament.mindmap.json`, on each of the three bases
    /// one could reasonably measure — the endpoints' baked
    /// `style.frame_color`, and the palette frame this cascade
    /// actually resolves, under each of the two level rules:
    ///
    /// | basis | `from_id` | `to_id` | neither | both |
    /// |---|---|---|---|---|
    /// | `style.frame_color` | 229 | 5 | 17 | 3 |
    /// | resolved palette, clamp (what this code does) | 145 | 14 | 103 | 14 |
    /// | resolved palette, wrap | 194 | 0 | 54 | 0 |
    ///
    /// Rows do not sum to 248 because an edge can match both
    /// endpoints or neither. Excluding the ties, the margin is 226
    /// against 2, 131 against 0, and 194 against 0: the conclusion
    /// survives every basis, which is why it is stated as one. The
    /// middle row is the one this code produces, and it is the
    /// weakest of the three — the baked `style` values the first
    /// row measures have drifted from the palettes they came from,
    /// which is the whole reason this cascade had to be wired.
    ///
    /// A cross-link is colored by the same rule, so the direction
    /// the author drew it in is the direction it takes its color
    /// from.
    ///
    /// The **frame** channel is the one used, not `background`: a
    /// connection is a stroke, and the frame is the palette's
    /// stroke color for that depth. It is
    /// [`Self::node_frame_theme_tier`] — the same reader the node's
    /// own border cascade sits on — so the source node's
    /// `overrides.frame` reaches the edges leaving it, exactly as it
    /// reaches the node's border. The two ladders read one channel;
    /// they must not read it at different tiers. Ordered the other
    /// way round, `color border=#ff0000` on a themed node would
    /// repaint its frame and silently leave its branch strokes on
    /// the palette's, with nothing on screen to explain the split —
    /// and the override tier exists precisely because a per-node
    /// write is more specific than the theme it excepts.
    ///
    /// Sharing the reader also shares its `non_empty` skip: a group
    /// (or override) whose `frame` is empty is a hole rather than a
    /// color, so the edge falls through to the canvas default and
    /// then to `edge.color` instead of handing the empty string to
    /// `hex_to_rgba_safe` and stroking opaque black.
    ///
    /// Cost: one node lookup plus one
    /// [`Self::node_frame_theme_tier`]. No allocation.
    pub fn edge_theme_stroke_color<'a>(&'a self, edge: &'a MindEdge) -> Option<&'a str> {
        let from = self.nodes.get(&edge.from_id)?;
        if !from
            .color_schema
            .as_ref()
            .is_some_and(|schema| schema.connections_colored)
        {
            return None;
        }
        self.node_frame_theme_tier(from)
    }

    /// [`MindEdge::body_color`] with this map's theme tier already
    /// supplied — the spelling every projection pass should use.
    ///
    /// Cost: [`Self::edge_theme_stroke_color`] plus the edge
    /// cascade's own O(1).
    pub fn edge_body_color<'a>(&'a self, edge: &'a MindEdge) -> &'a str {
        edge.body_color(&self.canvas, self.edge_theme_stroke_color(edge))
    }

    /// [`MindEdge::label_color`] with this map's theme tier already
    /// supplied. Cost: as [`Self::edge_body_color`].
    pub fn edge_label_color<'a>(&'a self, edge: &'a MindEdge) -> &'a str {
        edge.label_color(&self.canvas, self.edge_theme_stroke_color(edge))
    }

    /// [`MindEdge::portal_endpoint_color`] with this map's theme
    /// tier already supplied. `endpoint` is the state for the side
    /// being drawn — resolve it with
    /// [`portal_endpoint_state`](super::portal_endpoint_state)
    /// first. Cost: as [`Self::edge_body_color`].
    pub fn edge_portal_endpoint_color<'a>(
        &'a self,
        edge: &'a MindEdge,
        endpoint: Option<&'a super::PortalEndpointState>,
    ) -> &'a str {
        edge.portal_endpoint_color(&self.canvas, endpoint, self.edge_theme_stroke_color(edge))
    }

    /// [`MindEdge::portal_endpoint_text_color`] with this map's
    /// theme tier already supplied. Cost: as
    /// [`Self::edge_body_color`].
    pub fn edge_portal_endpoint_text_color<'a>(
        &'a self,
        edge: &'a MindEdge,
        endpoint: Option<&'a super::PortalEndpointState>,
    ) -> &'a str {
        edge.portal_endpoint_text_color(&self.canvas, endpoint, self.edge_theme_stroke_color(edge))
    }
}

#[cfg(test)]
mod tests {
    use crate::mindmap::model::{ColorGroup, ColorOverrides, ColorSchema, MindMap, Palette};
    use crate::mindmap::test_helpers::{synthetic_edge, synthetic_map, synthetic_node_full};

    /// Three groups whose every channel is a distinct sentinel, so a
    /// reader that grabs the wrong one cannot accidentally pass.
    fn probe_palette() -> Palette {
        Palette {
            groups: vec![
                group("#a10000", "#a20000", "#a30000", "#a40000"),
                group("#b10000", "#b20000", "#b30000", "#b40000"),
                group("#c10000", "#c20000", "#c30000", ""),
            ],
        }
    }

    fn group(background: &str, frame: &str, text: &str, title: &str) -> ColorGroup {
        ColorGroup {
            background: background.into(),
            frame: frame.into(),
            text: text.into(),
            title: title.into(),
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

    /// [`schema`] with this node's own exceptions to the group
    /// attached. `starts_at_root` is always `true` here — the
    /// override tier sits above the group lookup and does not
    /// interact with the level shift, which
    /// `test_starts_at_root_*` already covers.
    fn schema_overriding(level: usize, connections_colored: bool, overrides: ColorOverrides) -> ColorSchema {
        ColorSchema {
            overrides,
            ..schema(level, true, connections_colored)
        }
    }

    /// Two nodes joined `a -> b`, both carrying the sentinel styles
    /// `synthetic_node_full` pins (`#000` fill, `#fff` frame and
    /// text) so any palette value is distinguishable from the
    /// fallback.
    fn probe_map() -> MindMap {
        let mut map = synthetic_map(
            vec![
                synthetic_node_full("a", None, 0.0, 0.0, 80.0, 40.0, true),
                synthetic_node_full("b", Some("a"), 200.0, 0.0, 80.0, 40.0, true),
            ],
            vec![synthetic_edge("a", "b", "right", "left")],
        );
        map.palettes.insert("probe".into(), probe_palette());
        map
    }

    #[test]
    fn test_resolve_theme_colors_indexes_the_group_at_level() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(1, true, false));
        let node = map.nodes.get("a").unwrap();
        let resolved = map.resolve_theme_colors(node).expect("level 1 resolves");
        assert_eq!(resolved.frame, "#b20000");
    }

    #[test]
    fn test_resolve_theme_colors_none_without_a_schema() {
        let map = probe_map();
        let node = map.nodes.get("a").unwrap();
        assert!(map.resolve_theme_colors(node).is_none());
    }

    #[test]
    fn test_resolve_theme_colors_none_for_a_missing_palette() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(ColorSchema {
            palette: "no-such-palette".into(),
            level: 0,
            starts_at_root: true,
            connections_colored: false,
            overrides: ColorOverrides::default(),
        });
        let node = map.nodes.get("a").unwrap();
        assert!(
            map.resolve_theme_colors(node).is_none(),
            "a dangling palette reference degrades to the style fallback"
        );
    }

    #[test]
    fn test_resolve_theme_colors_none_for_an_empty_palette() {
        let mut map = probe_map();
        map.palettes
            .insert("probe".into(), Palette { groups: Vec::new() });
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
        let node = map.nodes.get("a").unwrap();
        assert!(map.resolve_theme_colors(node).is_none());
    }

    /// The clamp branch: a level past the last group lands on the
    /// last group rather than wrapping to `groups[0]` or panicking.
    #[test]
    fn test_resolve_theme_colors_clamps_a_level_past_the_last_group() {
        let mut map = probe_map();
        for level in [3usize, 4, 9, usize::MAX] {
            map.nodes.get_mut("a").unwrap().color_schema = Some(schema(level, true, false));
            let node = map.nodes.get("a").unwrap();
            let resolved = map
                .resolve_theme_colors(node)
                .expect("an out-of-range level still resolves");
            assert_eq!(
                resolved.frame, "#c20000",
                "level {level} must clamp to the last group, not wrap"
            );
        }
    }

    /// `starts_at_root = false` leaves the schema root itself
    /// unthemed and shifts its children down one group.
    #[test]
    fn test_starts_at_root_false_leaves_the_root_transparent() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, false, false));
        let node = map.nodes.get("a").unwrap();
        assert!(
            map.resolve_theme_colors(node).is_none(),
            "level 0 under starts_at_root = false is the transparent root"
        );
        assert_eq!(
            map.node_background_color(node),
            "#000",
            "and it keeps its own style fill"
        );
    }

    #[test]
    fn test_starts_at_root_false_shifts_children_onto_group_zero() {
        let mut map = probe_map();
        map.nodes.get_mut("b").unwrap().color_schema = Some(schema(1, false, false));
        let node = map.nodes.get("b").unwrap();
        let resolved = map.resolve_theme_colors(node).expect("level 1 resolves");
        assert_eq!(resolved.frame, "#a20000", "level 1 - 1 = groups[0]");
    }

    #[test]
    fn test_starts_at_root_true_and_false_disagree_by_one_group() {
        let mut map = probe_map();
        map.nodes.get_mut("b").unwrap().color_schema = Some(schema(2, true, false));
        let with_root = map
            .resolve_theme_colors(map.nodes.get("b").unwrap())
            .unwrap()
            .frame
            .clone();
        map.nodes.get_mut("b").unwrap().color_schema = Some(schema(2, false, false));
        let without_root = map
            .resolve_theme_colors(map.nodes.get("b").unwrap())
            .unwrap()
            .frame
            .clone();
        assert_eq!(with_root, "#c20000");
        assert_eq!(without_root, "#b20000");
    }

    /// Each role reads its own channel, and each falls back to its
    /// own `style` field when the node is unthemed.
    #[test]
    fn test_node_color_roles_read_their_own_channel() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
        let node = map.nodes.get("a").unwrap();
        assert_eq!(map.node_background_color(node), "#a10000");
        assert_eq!(map.node_frame_color(node), "#a20000");
        assert_eq!(map.node_text_color(node), "#a30000");
        assert_eq!(map.node_title_color(node), "#a40000");
    }

    #[test]
    fn test_node_color_roles_fall_back_to_style_without_a_schema() {
        let map = probe_map();
        let node = map.nodes.get("a").unwrap();
        assert_eq!(map.node_background_color(node), "#000");
        assert_eq!(map.node_frame_color(node), "#fff");
        assert_eq!(map.node_text_color(node), "#fff");
        assert_eq!(
            map.node_title_color(node),
            "#fff",
            "no style field carries a title color, so it follows text"
        );
    }

    /// The top tier of all four ladders: a channel this node names
    /// for itself beats the group it excepts, because a theme is
    /// inherited and a per-node edit is specific.
    #[test]
    fn test_node_color_roles_prefer_this_nodes_own_override() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema_overriding(
            0,
            false,
            ColorOverrides {
                background: Some("#010101".into()),
                frame: Some("#020202".into()),
                text: Some("#030303".into()),
                title: Some("#040404".into()),
            },
        ));
        let node = map.nodes.get("a").unwrap();
        assert_eq!(map.node_background_color(node), "#010101");
        assert_eq!(map.node_frame_color(node), "#020202");
        assert_eq!(map.node_text_color(node), "#030303");
        assert_eq!(map.node_title_color(node), "#040404");
        assert_eq!(
            map.node_frame_theme_tier(node),
            Some("#020202"),
            "and the border cascade sees the override as the node's theme tier"
        );
    }

    /// The override is per *channel*, not per node: the three
    /// channels this node says nothing about stay on the group, so
    /// a later palette edit still reaches them.
    #[test]
    fn test_an_override_leaves_the_channels_it_does_not_name_on_the_group() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema_overriding(
            0,
            false,
            ColorOverrides {
                background: Some("#010101".into()),
                ..ColorOverrides::default()
            },
        ));
        let node = map.nodes.get("a").unwrap();
        assert_eq!(map.node_background_color(node), "#010101");
        assert_eq!(map.node_frame_color(node), "#a20000");
        assert_eq!(map.node_text_color(node), "#a30000");
        assert_eq!(map.node_title_color(node), "#a40000");
    }

    /// An override stands even when the schema it excepts is
    /// broken — "I painted this node green" does not stop being
    /// true when the palette the theme names goes missing.
    #[test]
    fn test_an_override_survives_a_palette_that_does_not_resolve() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(ColorSchema {
            palette: "no-such-palette".into(),
            level: 0,
            starts_at_root: true,
            connections_colored: true,
            overrides: ColorOverrides {
                background: Some("#010101".into()),
                frame: Some("#020202".into()),
                text: Some("#030303".into()),
                title: Some("#040404".into()),
            },
        });
        let node = map.nodes.get("a").unwrap();
        assert!(map.resolve_theme_colors(node).is_none());
        assert_eq!(map.node_background_color(node), "#010101");
        assert_eq!(map.node_frame_color(node), "#020202");
        assert_eq!(map.node_text_color(node), "#030303");
        assert_eq!(map.node_title_color(node), "#040404");
        assert_eq!(
            map.edge_theme_stroke_color(&map.edges[0]),
            Some("#020202"),
            "and the edges leaving it take the override, not the palette that is not there"
        );
    }

    /// `title` is the one override channel no interactive setter
    /// writes — a hand-authored file is the only way it arrives —
    /// and it moves the first line without disturbing the rest of
    /// the section's text.
    #[test]
    fn test_a_title_override_moves_the_first_line_alone() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema_overriding(
            0,
            false,
            ColorOverrides {
                title: Some("#040404".into()),
                ..ColorOverrides::default()
            },
        ));
        let node = map.nodes.get("a").unwrap();
        assert_eq!(map.node_title_color(node), "#040404");
        assert_eq!(
            map.node_text_color(node),
            "#a30000",
            "the rest of the section stays on the group's text channel"
        );
    }

    /// …including where the group leaves `title` empty, the case
    /// the title role otherwise answers by following `text`. The
    /// override outranks that fallthrough, so a palette with no
    /// title channel can still have one node divide its first line.
    #[test]
    fn test_a_title_override_outranks_the_fallthrough_to_text() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema_overriding(
            2,
            false,
            ColorOverrides {
                title: Some("#040404".into()),
                ..ColorOverrides::default()
            },
        ));
        let node = map.nodes.get("a").unwrap();
        assert_eq!(map.node_title_color(node), "#040404");
        assert_eq!(map.node_text_color(node), "#c30000");
    }

    /// The `non_empty` skip on the **override** tier. No setter
    /// writes an empty override — they clear the channel instead —
    /// but `format/palettes.md` says a hand-authored file may carry
    /// one, and there it is a hole rather than a color for exactly
    /// the same three channels as in the group: `frame`, `text` and
    /// `title` fall through to the group underneath, while an empty
    /// `background` is the format's `transparent` and is passed
    /// along.
    #[test]
    fn test_an_empty_override_channel_falls_through_except_for_background() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema_overriding(
            0,
            true,
            ColorOverrides {
                background: Some(String::new()),
                frame: Some(String::new()),
                text: Some(String::new()),
                title: Some(String::new()),
            },
        ));
        let node = map.nodes.get("a").unwrap();
        assert_eq!(map.node_frame_color(node), "#a20000");
        assert_eq!(map.node_text_color(node), "#a30000");
        assert_eq!(map.node_title_color(node), "#a40000");
        assert_eq!(
            map.node_background_color(node),
            "",
            "an empty background is a value on the override tier as much as on the group"
        );
        assert_eq!(
            map.edge_theme_stroke_color(&map.edges[0]),
            Some("#a20000"),
            "and the edge ladder skips the hole with the border ladder"
        );
    }

    /// An empty channel is a *hole*, not a color — for every
    /// channel except `background`, where empty spells
    /// "transparent". A group leaving `text` or `frame` empty used
    /// to hand the empty string straight to `hex_to_rgba_safe`,
    /// which paints opaque black: a documented-legal palette value
    /// silently discarding the `style` value it shadows, while the
    /// identically-empty `title` fell through correctly.
    #[test]
    fn test_an_empty_group_channel_falls_through_except_for_background() {
        let mut map = probe_map();
        map.palettes.insert(
            "holes".into(),
            Palette {
                groups: vec![group("", "", "", "")],
            },
        );
        {
            let node = map.nodes.get_mut("a").unwrap();
            node.color_schema = Some(ColorSchema {
                palette: "holes".into(),
                level: 0,
                starts_at_root: true,
                connections_colored: true,
                overrides: ColorOverrides::default(),
            });
        }
        let node = map.nodes.get("a").unwrap();
        assert_eq!(
            map.node_frame_color(node),
            "#fff",
            "empty frame falls through to style"
        );
        assert_eq!(
            map.node_text_color(node),
            "#fff",
            "empty text falls through to style"
        );
        assert_eq!(map.node_title_color(node), "#fff", "…and title follows text");
        assert_eq!(
            map.node_background_color(node),
            "",
            "an empty background is the format's `transparent`, and is passed along"
        );
        assert_eq!(
            map.node_frame_theme_tier(node),
            None,
            "a hole is not a theme tier, so a canvas-wide border color still applies"
        );
    }

    #[test]
    fn test_node_title_color_falls_back_to_text_when_the_group_leaves_it_empty() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(2, true, false));
        let node = map.nodes.get("a").unwrap();
        assert_eq!(map.node_title_color(node), "#c30000");
    }

    /// The headline claim in `format/palettes.md`: editing the
    /// palette re-themes every node bound to it, with no per-node
    /// edit.
    #[test]
    fn test_editing_the_palette_changes_every_bound_node() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
        map.nodes.get_mut("b").unwrap().color_schema = Some(schema(0, true, false));
        assert_eq!(map.node_background_color(map.nodes.get("a").unwrap()), "#a10000");
        assert_eq!(map.node_background_color(map.nodes.get("b").unwrap()), "#a10000");

        map.palettes.get_mut("probe").unwrap().groups[0].background = "#0f0f0f".into();

        assert_eq!(map.node_background_color(map.nodes.get("a").unwrap()), "#0f0f0f");
        assert_eq!(map.node_background_color(map.nodes.get("b").unwrap()), "#0f0f0f");
    }

    #[test]
    fn test_edge_theme_stroke_color_follows_the_source_node() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, true));
        map.nodes.get_mut("b").unwrap().color_schema = Some(schema(1, true, true));
        assert_eq!(
            map.edge_theme_stroke_color(&map.edges[0]),
            Some("#a20000"),
            "the edge takes its from_id node's group, not its to_id node's"
        );
    }

    /// The frame channel is read at **one** tier by both ladders.
    /// `edge_theme_stroke_color` used to reach for the palette
    /// group's `frame` directly, so `color border=#020202` on a
    /// themed node repainted its border and left every edge leaving
    /// it stroking the group's color, with nothing on screen to
    /// explain the split.
    #[test]
    fn test_a_frame_override_reaches_the_edges_leaving_the_node() {
        let mut map = probe_map();
        map.edges[0].color = "#edge00".into();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema_overriding(
            0,
            true,
            ColorOverrides {
                frame: Some("#020202".into()),
                ..ColorOverrides::default()
            },
        ));
        assert_eq!(map.edge_theme_stroke_color(&map.edges[0]), Some("#020202"));
        assert_eq!(map.edge_body_color(&map.edges[0]), "#020202");
    }

    /// …and they agree tier for tier, not just on that one case:
    /// whatever the node's border cascade is handed as its theme
    /// tier is what the edges leaving it stroke with.
    #[test]
    fn test_the_border_and_connection_ladders_read_the_frame_channel_alike() {
        let mut map = probe_map();
        let overrides = [
            ColorOverrides::default(),
            ColorOverrides {
                frame: Some("#020202".into()),
                ..ColorOverrides::default()
            },
            ColorOverrides {
                frame: Some(String::new()),
                ..ColorOverrides::default()
            },
            ColorOverrides {
                background: Some("#010101".into()),
                text: Some("#030303".into()),
                ..ColorOverrides::default()
            },
        ];
        for (index, overrides) in overrides.into_iter().enumerate() {
            for level in [0usize, 1, 2, 7] {
                map.nodes.get_mut("a").unwrap().color_schema =
                    Some(schema_overriding(level, true, overrides.clone()));
                let node = map.nodes.get("a").unwrap();
                assert_eq!(
                    map.node_frame_theme_tier(node),
                    map.edge_theme_stroke_color(&map.edges[0]),
                    "overrides #{index} at level {level}: the border and connection \
                     ladders must read the frame channel at the same tier"
                );
            }
        }
    }

    /// A group whose `frame` is empty is a hole for the edge ladder
    /// too, so the stroke falls through to the tiers underneath
    /// rather than handing the empty string to `hex_to_rgba_safe`
    /// and painting opaque black.
    #[test]
    fn test_an_empty_frame_is_not_a_stroke_color_for_the_edges_leaving_it() {
        let mut map = probe_map();
        map.edges[0].color = "#edge00".into();
        map.palettes.insert(
            "holes".into(),
            Palette {
                groups: vec![group("", "", "", "")],
            },
        );
        map.nodes.get_mut("a").unwrap().color_schema = Some(ColorSchema {
            palette: "holes".into(),
            ..schema(0, true, true)
        });
        assert!(map.edge_theme_stroke_color(&map.edges[0]).is_none());
        assert_eq!(map.edge_body_color(&map.edges[0]), "#edge00");
    }

    #[test]
    fn test_edge_theme_stroke_color_none_when_connections_are_not_colored() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
        assert!(map.edge_theme_stroke_color(&map.edges[0]).is_none());
    }

    #[test]
    fn test_edge_theme_stroke_color_none_when_the_source_is_unthemed() {
        let map = probe_map();
        assert!(map.edge_theme_stroke_color(&map.edges[0]).is_none());
    }

    #[test]
    fn test_edge_body_color_prefers_the_theme_over_the_edges_own_color() {
        let mut map = probe_map();
        map.edges[0].color = "#edge00".into();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, true));
        assert_eq!(map.edge_body_color(&map.edges[0]), "#a20000");
        // …and gives it back the moment the flag comes off.
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
        assert_eq!(map.edge_body_color(&map.edges[0]), "#edge00");
    }

    #[test]
    fn test_an_explicit_connection_color_outranks_the_theme() {
        use crate::mindmap::model::GlyphConnectionConfig;
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, true));
        map.edges[0].glyph_connection = Some(GlyphConnectionConfig {
            color: Some("#explicit".into()),
            ..GlyphConnectionConfig::default()
        });
        assert_eq!(
            map.edge_body_color(&map.edges[0]),
            "#explicit",
            "a color the author named on the edge is not a theme's to overrule"
        );
    }

    /// The tier that had been sitting in the wrong place: a
    /// **map-wide** connection default is less specific than a
    /// **per-node** theme, so the theme wins. Ordered the other way
    /// round, one `canvas connection color=…` flattens every
    /// palette-derived stroke in the document and no per-edge edit
    /// short of forking a `glyph_connection` recovers one.
    #[test]
    fn test_the_theme_outranks_the_canvas_wide_connection_default() {
        use crate::mindmap::model::GlyphConnectionConfig;
        let mut map = probe_map();
        map.edges[0].color = "#edge00".into();
        map.canvas.default_connection = Some(GlyphConnectionConfig {
            color: Some("#888888".into()),
            ..GlyphConnectionConfig::default()
        });
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, true));
        assert_eq!(map.edge_body_color(&map.edges[0]), "#a20000");
        // Without a theme the canvas default is still the tier
        // above `edge.color` — the reorder moved it, it did not
        // remove it.
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
        assert_eq!(map.edge_body_color(&map.edges[0]), "#888888");
    }

    /// The struct-level fork rule survives the reorder: an edge
    /// that has forked a `glyph_connection` of its own never
    /// consults the canvas default again, even for a field the
    /// fork left at `None`.
    #[test]
    fn test_a_forked_connection_still_ignores_the_canvas_default() {
        use crate::mindmap::model::GlyphConnectionConfig;
        let mut map = probe_map();
        map.edges[0].color = "#edge00".into();
        map.canvas.default_connection = Some(GlyphConnectionConfig {
            color: Some("#888888".into()),
            ..GlyphConnectionConfig::default()
        });
        map.edges[0].glyph_connection = Some(GlyphConnectionConfig {
            color: None,
            ..GlyphConnectionConfig::default()
        });
        assert_eq!(
            map.edge_body_color(&map.edges[0]),
            "#edge00",
            "a forked edge with no color of its own falls to `edge.color`, not the canvas default"
        );
    }

    /// The border channel has the same shape and now the same
    /// order: an explicit per-node `border.color` wins, the node's
    /// theme outranks the canvas-wide default border, and
    /// `style.frame_color` is the floor.
    #[test]
    fn test_the_theme_outranks_the_canvas_wide_default_border() {
        use crate::mindmap::border::resolve_border_style;
        use crate::mindmap::model::GlyphBorderConfig;
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
        let canvas_default = GlyphBorderConfig {
            preset: "light".into(),
            font: None,
            font_size_pt: 14.0,
            color: Some("#888888".into()),
            glyphs: None,
            padding: 4.0,
            color_palette: None,
            color_palette_field: None,
        };
        let node = map.nodes.get("a").unwrap();
        let resolved = resolve_border_style(
            node.style.border.as_ref(),
            Some(&canvas_default),
            map.node_frame_theme_tier(node),
            &node.style.frame_color,
        );
        assert_eq!(
            resolved.color, "#a20000",
            "the node's palette frame beats a map-wide border color"
        );

        // Unthemed, the canvas default is still the tier above
        // `style.frame_color`.
        map.nodes.get_mut("a").unwrap().color_schema = None;
        let node = map.nodes.get("a").unwrap();
        let resolved = resolve_border_style(
            node.style.border.as_ref(),
            Some(&canvas_default),
            map.node_frame_theme_tier(node),
            &node.style.frame_color,
        );
        assert_eq!(resolved.color, "#888888");
    }

    #[test]
    fn test_edge_label_and_portal_channels_inherit_the_theme_through_the_body() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, true));
        let edge = &map.edges[0];
        assert_eq!(map.edge_label_color(edge), "#a20000");
        assert_eq!(map.edge_portal_endpoint_color(edge, None), "#a20000");
        assert_eq!(map.edge_portal_endpoint_text_color(edge, None), "#a20000");
    }
}
