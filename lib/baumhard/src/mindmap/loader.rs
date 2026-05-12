// SPDX-License-Identifier: MPL-2.0

//! `.mindmap.json` loader + saver. Accepts both the current edge-based
//! portal shape and pre-refactor files that still ship a top-level
//! `portals[]` array, rejecting the latter with a concrete migration
//! pointer instead of silently dropping data.

use crate::mindmap::model::MindMap;
use std::fs;
use std::path::Path;

/// Load a `MindMap` from a file path. Reads the entire file into
/// memory via `std::fs::read_to_string`, then delegates to
/// [`load_from_str`]. Native-only (synchronous I/O). Returns a
/// `String` error describing the path + underlying cause.
///
/// Cost: one filesystem read (latency-bound, plus an allocation
/// sized to the file's UTF-8 length) followed by [`load_from_str`]'s
/// JSON parse — O(file_size) overall. Felt every map load.
pub fn load_from_file(path: &Path) -> Result<MindMap, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read file {}: {}", path.display(), e))?;
    load_from_str(&content)
}

/// Parse a `MindMap` from a JSON string. Rejects pre-refactor files
/// that still carry a top-level `portals[]` array, or per-node
/// `text` / `text_runs` instead of `sections[]`, with concrete
/// migration pointers (`maptool convert ...`) so a stale map
/// doesn't silently lose its content — serde would otherwise
/// ignore the unknown fields.
///
/// Cost: one typed parse on the happy path. The legacy-shape
/// detection (`Value` walk) runs only when the typed parse
/// fails OR when the typed result has zero-section nodes
/// (`sections: []` is silently legal for serde but a current-
/// invariant violation we surface as a migration hint) OR when
/// the cheap substring screen flags a dropped field. The
/// substring screen requires the marker to be followed by `":`
/// so node text containing the literal word "portals" or
/// "text_runs" doesn't false-positive — JSON keys always have
/// `":` after them.
pub fn load_from_str(json: &str) -> Result<MindMap, String> {
    match serde_json::from_str::<MindMap>(json) {
        Ok(map) => {
            // Post-typed-parse invariants serde can't enforce:
            // - empty `sections[]` (every node needs one section);
            // - silently-dropped legacy `portals` / `text` /
            //   `text_runs` fields. We only inspect the raw JSON
            //   here when the typed map carries the symptom
            //   (zero-section node, or the substring marker
            //   indicates a dropped field).
            if map.nodes.values().any(|n| n.sections.is_empty()) || has_legacy_marker(json) {
                if let Some(err) = detect_legacy_shape(json) {
                    return Err(err);
                }
            }
            Ok(map)
        }
        Err(e) => {
            // Typed parse failed. Try the Value-based legacy
            // detector — if it fingers a known legacy shape,
            // surface the migration pointer; otherwise fall
            // through to the raw serde error so the caller sees
            // the actual parse failure (line / column included).
            if let Some(err) = detect_legacy_shape(json) {
                return Err(err);
            }
            Err(format!("Failed to parse mindmap JSON: {}", e))
        }
    }
}

/// Cheap pre-screen for the substring patterns that legacy JSON
/// keys produce. Requires the trailing `":` so user text
/// containing the literal word `"portals"` (e.g. a node titled
/// `My "portals"`) doesn't trigger the deeper [`detect_legacy_shape`]
/// pass — JSON keys always emit as `"key":`, while string
/// content emits as `\"key\"` with escape backslashes that
/// break the substring match.
fn has_legacy_marker(json: &str) -> bool {
    json.contains("\"portals\":") || json.contains("\"text_runs\":")
}

/// Inspect `json` for legacy field shapes that the typed parse
/// would silently drop or misread, returning a migration-pointing
/// error message on first hit. Walks `serde_json::Value` once;
/// only called when the cheap substring screen suggests a legacy
/// marker is present.
fn detect_legacy_shape(json: &str) -> Option<String> {
    let raw: serde_json::Value = serde_json::from_str(json).ok()?;
    // Pre-refactor maps stored portals in a separate `portals[]` array.
    // Post-refactor portals are edges with `display_mode = "portal"`.
    if let Some(arr) = raw.get("portals").and_then(|p| p.as_array()) {
        if !arr.is_empty() {
            return Some(
                "legacy `portals` field present; run `maptool convert --portals <file>` \
                 to migrate to portal-mode edges"
                    .to_string(),
            );
        }
    }
    // Pre-section-refactor maps put `text` and `text_runs` directly
    // on each node. Post-refactor those live on
    // `MindNode.sections[].{text, text_runs}`.
    if let Some(nodes) = raw.get("nodes").and_then(|n| n.as_object()) {
        if let Some((id, _)) = nodes
            .iter()
            .find(|(_, v)| v.get("text").is_some() || v.get("text_runs").is_some())
        {
            return Some(format!(
                "legacy `text` / `text_runs` on node {:?}; run \
                 `maptool convert --sections <file>` to migrate node \
                 text into `sections[]`",
                id
            ));
        }
        if let Some((id, _)) = nodes
            .iter()
            .find(|(_, v)| v.get("sections").map(|s| !s.is_array()).unwrap_or(false))
        {
            return Some(format!(
                "node {:?} has `sections` but it is not an array — \
                 see format/sections.md",
                id
            ));
        }
        if let Some((id, _)) = nodes.iter().find(|(_, v)| {
            v.get("sections")
                .and_then(|s| s.as_array())
                .map(|arr| arr.is_empty())
                .unwrap_or(true)
        }) {
            return Some(format!(
                "node {:?} ships zero sections — every renderable node \
                 needs at least one. Run `maptool convert --sections <file>` \
                 to migrate, or add an explicit `sections` array.",
                id
            ));
        }
    }
    None
}

/// Serialize a `MindMap` to pretty-printed JSON and write it to disk
/// atomically and deterministically.
///
/// **Determinism**: routes through `serde_json::Value` (which uses
/// `BTreeMap` for object keys) so two saves of the same `MindMap` produce
/// byte-identical output regardless of `HashMap` iteration order. Costs
/// one extra heap copy of the JSON tree; acceptable for the editor's
/// save cadence (post-mutation, not per-frame).
///
/// **Atomicity**: writes to a sibling `.<name>.<pid>.tmp` file then
/// renames over `path`. A reader (another process, or the editor
/// reloading after an external edit) never observes a torn-write
/// half-written file. The temp file is removed on rename failure.
///
/// Native-only (synchronous I/O via `std::fs`). Returns a `String`
/// error describing the path + underlying cause.
pub fn save_to_file(path: &Path, map: &MindMap) -> Result<(), String> {
    let value = serde_json::to_value(map).map_err(|e| format!("failed to serialize map: {e}"))?;
    let json = serde_json::to_string_pretty(&value).map_err(|e| format!("failed to render map JSON: {e}"))?;
    write_atomic(path, &json)
}

/// Write `contents` to `path` via `<dir>/.<name>.<pid>.tmp` + rename.
/// Cleans up the temp file on rename failure so a partially-written
/// staging file is never left behind. Used by [`save_to_file`] for the
/// typed-`MindMap` save path; also exposed for legacy-migration tools
/// (`maptool convert --portals` etc.) that ship raw `serde_json::Value`
/// to disk without a `MindMap` round-trip.
pub fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("invalid path: {}", path.display()))?
        .to_string_lossy();
    let tmp_path = dir.join(format!(".{}.{}.tmp", file_name, std::process::id()));
    fs::write(&tmp_path, contents).map_err(|e| format!("failed to write {}: {e}", tmp_path.display()))?;
    fs::rename(&tmp_path, path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!(
            "failed to rename {} -> {}: {e}",
            tmp_path.display(),
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mindmap::test_helpers::testament_map_path as test_map_path;
    use std::path::PathBuf;

    #[test]
    fn test_load_testament_map() {
        let path = test_map_path();
        let map = load_from_file(&path).expect("Failed to load testament map");

        assert_eq!(map.version, "1.0");
        assert_eq!(map.name, "testament");
        assert_eq!(map.canvas.background_color, "#000000");
        assert_eq!(map.nodes.len(), 252);
        assert_eq!(map.edges.len(), 258);
    }

    #[test]
    fn test_root_nodes() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let roots = map.root_nodes();
        assert!(!roots.is_empty());
        for root in &roots {
            assert!(root.parent_id.is_none());
        }
        // Verify sorted by index
        for w in roots.windows(2) {
            assert!(
                crate::mindmap::model::id_sort_key(&w[0].id) <= crate::mindmap::model::id_sort_key(&w[1].id)
            );
        }
    }

    #[test]
    fn test_children_of() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        // Lord God node
        let children = map.children_of("0");
        assert!(!children.is_empty());
        for child in &children {
            assert_eq!(child.parent_id.as_deref(), Some("0"));
        }
        // Verify sorted by index
        for w in children.windows(2) {
            assert!(
                crate::mindmap::model::id_sort_key(&w[0].id) <= crate::mindmap::model::id_sort_key(&w[1].id)
            );
        }
    }

    /// Pre-section-refactor maps carry `text` / `text_runs` directly
    /// on each node; the loader rejects those with a concrete
    /// migration pointer (per CODE_CONVENTIONS §10 "no dual shapes")
    /// instead of silently dropping the unknown fields. Mirrors the
    /// portal-legacy rejection at the top of `load_from_str`.
    #[test]
    fn test_legacy_text_field_rejected_with_migration_pointer() {
        let raw = r##"{
            "version": "1.0",
            "name": "legacy",
            "canvas": {"background_color": "#000", "default_border": null,
                       "default_connection": null, "theme_variables": {},
                       "theme_variants": {}},
            "nodes": {"0": {
                "id": "0", "parent_id": null,
                "position": {"x": 0, "y": 0},
                "size": {"width": 100, "height": 50},
                "text": "I'm legacy",
                "text_runs": [],
                "style": {"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false},
                "layout": {"type":"map","direction":"auto","spacing":0},
                "folded": false, "notes": "",
                "color_schema": null
            }},
            "edges": []
        }"##;
        let err = load_from_str(raw).expect_err("legacy text field must be rejected");
        assert!(
            err.contains("legacy") && err.contains("maptool convert --sections"),
            "error must point at the migration tool: {err}"
        );
    }

    /// A valid post-section node parses through the typed loader.
    /// Pairs with `test_legacy_text_field_rejected_with_migration_pointer`
    /// — the rejection only fires for maps that ship the legacy
    /// shape, not for fresh ones.
    #[test]
    fn test_post_section_node_parses() {
        let raw = r##"{
            "version": "1.0",
            "name": "fresh",
            "canvas": {"background_color": "#000", "default_border": null,
                       "default_connection": null, "theme_variables": {},
                       "theme_variants": {}},
            "nodes": {"0": {
                "id": "0", "parent_id": null,
                "position": {"x": 0, "y": 0},
                "size": {"width": 100, "height": 50},
                "sections": [{"text": "ok"}],
                "style": {"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false},
                "layout": {"type":"map","direction":"auto","spacing":0},
                "folded": false, "notes": "",
                "color_schema": null
            }},
            "edges": []
        }"##;
        let map = load_from_str(raw).expect("post-section node parses");
        assert_eq!(map.nodes.len(), 1);
        let node = map.nodes.get("0").unwrap();
        assert_eq!(node.sections.len(), 1);
        assert_eq!(node.sections[0].text, "ok");
    }

    /// A node with `sections: []` is rejected — every renderable
    /// node needs at least one section, and the loader catches this
    /// at parse time so the tree builder's recursion never sees a
    /// zero-section node.
    #[test]
    fn test_zero_sections_rejected() {
        let raw = r##"{
            "version": "1.0",
            "name": "empty",
            "canvas": {"background_color": "#000", "default_border": null,
                       "default_connection": null, "theme_variables": {},
                       "theme_variants": {}},
            "nodes": {"0": {
                "id": "0", "parent_id": null,
                "position": {"x": 0, "y": 0},
                "size": {"width": 100, "height": 50},
                "sections": [],
                "style": {"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false},
                "layout": {"type":"map","direction":"auto","spacing":0},
                "folded": false, "notes": "",
                "color_schema": null
            }},
            "edges": []
        }"##;
        let err = load_from_str(raw).expect_err("empty sections must be rejected");
        assert!(
            err.contains("zero sections"),
            "error must explain the invariant: {err}"
        );
    }

    #[test]
    fn test_text_runs() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let node = map.nodes.get("0").unwrap();
        assert_eq!(node.sections.len(), 1, "post-migration: one section per node");
        let section = &node.sections[0];
        assert_eq!(section.text, "Lord God");
        assert_eq!(section.text_runs.len(), 1);
        let run = &section.text_runs[0];
        assert_eq!(run.start, 0);
        assert_eq!(run.end, 8);
        assert!(run.bold);
        assert!(run.underline);
        assert_eq!(run.font, "LiberationSans");
        assert_eq!(run.size_pt, 74);
        assert_eq!(run.color, "#ffffff");
    }

    #[test]
    fn test_color_schema() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let root_node = map.nodes.get("0").unwrap();
        let schema = root_node.color_schema.as_ref().unwrap();
        assert_eq!(schema.level, 0);
        assert!(schema.palette.starts_with("coral"));
        let palette = map.palettes.get(&schema.palette).unwrap();
        assert!(!palette.groups.is_empty());
        assert_eq!(palette.groups[0].frame, "#30b082");
    }

    #[test]
    fn test_edges() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let edge = &map.edges[0];
        assert_eq!(edge.from_id, "0");
        assert_eq!(edge.to_id, "0.0");
        assert_eq!(edge.edge_type, "parent_child");
        assert!(edge.visible);

        // Find an edge with control points
        let curved = map.edges.iter().find(|e| !e.control_points.is_empty());
        assert!(curved.is_some());
    }

    #[test]
    fn test_resolve_theme_colors() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        // Root schema node should resolve to level 0 group
        let root_node = map.nodes.get("0").unwrap();
        let colors = map.resolve_theme_colors(root_node).unwrap();
        assert_eq!(colors.frame, "#30b082");
    }

    #[test]
    fn test_testament_edges_produce_paths() {
        use crate::mindmap::connection;

        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let mut straight_count = 0;
        let mut bezier_count = 0;
        for edge in &map.edges {
            let from_node = map.nodes.get(&edge.from_id).expect("Missing from_node");
            let to_node = map.nodes.get(&edge.to_id).expect("Missing to_node");

            let from_pos = from_node.pos_vec2();
            let from_size = from_node.size_vec2();
            let to_pos = to_node.pos_vec2();
            let to_size = to_node.size_vec2();

            let conn_path = connection::build_connection_path(
                from_pos,
                from_size,
                &edge.anchor_from,
                to_pos,
                to_size,
                &edge.anchor_to,
                &edge.control_points,
            );
            match conn_path {
                connection::ConnectionPath::Straight { .. } => straight_count += 1,
                connection::ConnectionPath::CubicBezier { .. } => bezier_count += 1,
            }

            // Verify sampling produces non-empty result
            let samples = connection::sample_path(&conn_path, 7.2);
            assert!(
                !samples.is_empty(),
                "Edge {}→{} produced no samples",
                edge.from_id,
                edge.to_id
            );
        }
        assert_eq!(straight_count + bezier_count, 258);
        assert!(straight_count > 200, "Expected most edges to be straight");
        assert!(bezier_count > 0, "Expected some Bezier edges");
    }

    #[test]
    fn test_testament_scene_has_connections() {
        use crate::mindmap::scene_builder;

        let path = test_map_path();
        let map = load_from_file(&path).unwrap();
        let scene = scene_builder::build_scene(&map, 1.0);

        // All visible edges should produce connection elements
        let visible_edges = map.edges.iter().filter(|e| e.visible).count();
        assert_eq!(
            scene.connection_elements.len(),
            visible_edges,
            "Expected {} connection elements, got {}",
            visible_edges,
            scene.connection_elements.len()
        );

        // Each connection element should have glyph positions
        for elem in &scene.connection_elements {
            assert!(
                !elem.glyph_positions.is_empty(),
                "Connection has no glyph positions"
            );
            assert!(!elem.body_glyph.is_empty(), "Connection has no body glyph");
            assert!(!elem.color.is_empty(), "Connection has no color");
        }
    }

    #[test]
    fn test_backward_compat_no_custom_mutations() {
        // Existing maps without custom_mutations/trigger_bindings/inline_mutations
        // should load with empty defaults
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        assert!(
            map.custom_mutations.is_empty(),
            "Existing map should have no custom_mutations"
        );

        let node = map.nodes.get("0").unwrap();
        assert!(
            node.trigger_bindings.is_empty(),
            "Existing node should have no trigger_bindings"
        );
        assert!(
            node.inline_mutations.is_empty(),
            "Existing node should have no inline_mutations"
        );
    }

    #[test]
    fn test_backward_compat_no_theme_variables() {
        // Existing maps without theme_variables/theme_variants should load
        // with empty defaults (the new fields must be opt-in via serde default).
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();
        assert!(map.canvas.theme_variables.is_empty());
        assert!(map.canvas.theme_variants.is_empty());
    }

    /// Existing fixtures pre-date the zoom-visibility feature, so
    /// every model field on every node / edge / label / portal
    /// endpoint must round-trip as `None`. Pins the
    /// `skip_serializing_if` contract so the on-disk form of
    /// unchanged maps stays byte-stable against the JSON keys
    /// — a regression in a serde attribute would surface here as
    /// a `Some(…)` drift on load, and (via the follow-up
    /// serialize roundtrip) as a newly-emitted key in the file.
    #[test]
    fn test_existing_fixtures_have_no_authored_zoom_windows() {
        let path = test_map_path();
        let map = load_from_file(&path).expect("testament loads");
        for node in map.nodes.values() {
            assert!(
                node.min_zoom_to_render.is_none(),
                "testament node {} has an unexpected min_zoom_to_render",
                node.id
            );
            assert!(
                node.max_zoom_to_render.is_none(),
                "testament node {} has an unexpected max_zoom_to_render",
                node.id
            );
        }
        for (i, edge) in map.edges.iter().enumerate() {
            assert!(
                edge.min_zoom_to_render.is_none(),
                "testament edge[{i}] has an unexpected min_zoom_to_render"
            );
            assert!(
                edge.max_zoom_to_render.is_none(),
                "testament edge[{i}] has an unexpected max_zoom_to_render"
            );
            if let Some(cfg) = edge.label_config.as_ref() {
                assert!(cfg.min_zoom_to_render.is_none());
                assert!(cfg.max_zoom_to_render.is_none());
            }
            if let Some(pf) = edge.portal_from.as_ref() {
                assert!(pf.min_zoom_to_render.is_none());
                assert!(pf.max_zoom_to_render.is_none());
                assert!(pf.perpendicular_offset.is_none());
            }
            if let Some(pt) = edge.portal_to.as_ref() {
                assert!(pt.min_zoom_to_render.is_none());
                assert!(pt.max_zoom_to_render.is_none());
                assert!(pt.perpendicular_offset.is_none());
            }
        }

        // Serialize back and confirm the raw JSON never mentions
        // the new keys — the `skip_serializing_if = "Option::is_none"`
        // attributes must suppress every default field.
        let serialized = serde_json::to_string(&map).expect("serializes");
        assert!(
            !serialized.contains("min_zoom_to_render"),
            "testament roundtrip emitted an unexpected min_zoom_to_render key"
        );
        assert!(
            !serialized.contains("max_zoom_to_render"),
            "testament roundtrip emitted an unexpected max_zoom_to_render key"
        );
        assert!(
            !serialized.contains("perpendicular_offset"),
            "testament roundtrip emitted an unexpected perpendicular_offset key"
        );
    }

    /// Roundtrip the same fixture through a second parse and
    /// confirm the structural shape is preserved — serde's
    /// default / skip_if attributes on the new fields must be
    /// symmetric so two load / save passes converge on the
    /// same model. Complements the raw-JSON check above.
    #[test]
    fn test_testament_double_roundtrip_is_stable() {
        let path = test_map_path();
        let first = load_from_file(&path).expect("first load");
        let intermediate = serde_json::to_string(&first).expect("first serialize");
        let second: MindMap = serde_json::from_str(&intermediate).expect("second load");

        // Canonical markers on the model that would drift if
        // any new serde attribute was asymmetric. Cover each of
        // the four structs that gained the zoom pair.
        assert_eq!(first.nodes.len(), second.nodes.len());
        assert_eq!(first.edges.len(), second.edges.len());
        for (id, first_node) in &first.nodes {
            let second_node = second.nodes.get(id).expect("node preserved");
            assert_eq!(first_node.min_zoom_to_render, second_node.min_zoom_to_render);
            assert_eq!(first_node.max_zoom_to_render, second_node.max_zoom_to_render);
        }
        for (first_edge, second_edge) in first.edges.iter().zip(second.edges.iter()) {
            assert_eq!(first_edge.min_zoom_to_render, second_edge.min_zoom_to_render);
            assert_eq!(first_edge.max_zoom_to_render, second_edge.max_zoom_to_render);
        }
    }

    fn theme_demo_path() -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop();
        path.pop();
        path.push("maps/theme_demo.mindmap.json");
        path
    }

    #[test]
    fn test_load_theme_demo_map() {
        let path = theme_demo_path();
        let map = load_from_file(&path).expect("Failed to load theme demo map");
        assert_eq!(map.version, "1.0");
        assert_eq!(map.name, "theme_demo");
        assert_eq!(map.canvas.background_color, "var(--bg)");
        assert!(map.canvas.theme_variables.contains_key("--bg"));
        assert_eq!(map.canvas.theme_variants.len(), 3);
        assert!(map.canvas.theme_variants.contains_key("dark"));
        assert!(map.canvas.theme_variants.contains_key("light"));
        assert!(map.canvas.theme_variants.contains_key("forest"));
        assert_eq!(map.custom_mutations.len(), 3);
    }

    #[test]
    fn test_theme_demo_scene_resolves_background() {
        use crate::mindmap::scene_builder;
        let path = theme_demo_path();
        let map = load_from_file(&path).unwrap();
        let scene = scene_builder::build_scene(&map, 1.0);
        // Background should resolve through the dark theme var set.
        assert_eq!(scene.background_color, "#141414");
    }

    #[test]
    fn test_theme_demo_roundtrip() {
        let path = theme_demo_path();
        let map = load_from_file(&path).unwrap();
        let json = serde_json::to_string(&map).unwrap();
        let back: MindMap = serde_json::from_str(&json).unwrap();
        assert_eq!(back.canvas.theme_variants.len(), 3);
        assert_eq!(back.custom_mutations.len(), 3);
    }

    /// Two consecutive `save_to_file` calls on the same `MindMap`
    /// produce byte-identical files. `MindMap.nodes` is a `HashMap`
    /// whose iteration order is randomised per-process; routing
    /// through `serde_json::Value` (a `BTreeMap` under the hood)
    /// pins the order. Without this, every save would diff against
    /// the previous one even when nothing changed.
    #[test]
    fn test_save_to_file_is_deterministic() {
        let map = load_from_file(&test_map_path()).unwrap();
        let dir = std::env::temp_dir();
        let path_a = dir.join("mandala_determinism_a.mindmap.json");
        let path_b = dir.join("mandala_determinism_b.mindmap.json");
        save_to_file(&path_a, &map).expect("save a failed");
        save_to_file(&path_b, &map).expect("save b failed");
        let bytes_a = std::fs::read(&path_a).unwrap();
        let bytes_b = std::fs::read(&path_b).unwrap();
        assert_eq!(bytes_a, bytes_b, "save output must be deterministic");
        let _ = std::fs::remove_file(&path_a);
        let _ = std::fs::remove_file(&path_b);
    }

    /// `save_to_file` writes to `<dir>/.<name>.<pid>.tmp` then renames
    /// over `path`; on a successful rename, the staging file is gone.
    /// Pins the contract that a kill mid-write leaves either the old
    /// file intact or the new file complete — never a torn partial
    /// write next to either.
    #[test]
    fn test_save_to_file_leaves_no_tmp_file_on_success() {
        let map = MindMap::new_blank("no-tmp");
        let dir = std::env::temp_dir();
        let path = dir.join("mandala_no_tmp_leftover.mindmap.json");
        save_to_file(&path, &map).expect("save failed");

        let pid = std::process::id();
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        let leftover = dir.join(format!(".{file_name}.{pid}.tmp"));
        assert!(
            !leftover.exists(),
            "atomic writer left a temp file behind: {}",
            leftover.display()
        );
        let _ = std::fs::remove_file(&path);
    }

    /// `save_to_file` → `load_from_file` reproduces the same `MindMap`
    /// for both the loaded testament fixture and a freshly-blank map
    /// (the `new` console verb lands on this). Locks the on-disk
    /// format as the canonical serialization for both shapes.
    #[test]
    fn test_save_to_file_round_trip_for_loaded_and_blank_maps() {
        let testament = load_from_file(&test_map_path()).unwrap();
        let blank = MindMap::new_blank("untitled");

        for (label, original) in [("testament", testament), ("blank", blank)] {
            let tmp = std::env::temp_dir().join(format!("mandala_save_round_trip_{label}.mindmap.json"));
            save_to_file(&tmp, &original).expect("save failed");
            let reloaded = load_from_file(&tmp).expect("reload failed");

            assert_eq!(reloaded.version, original.version, "{label}: version");
            assert_eq!(reloaded.name, original.name, "{label}: name");
            assert_eq!(reloaded.nodes.len(), original.nodes.len(), "{label}: nodes len");
            assert_eq!(reloaded.edges.len(), original.edges.len(), "{label}: edges len");
            assert_eq!(
                reloaded.canvas.background_color, original.canvas.background_color,
                "{label}: bg",
            );
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// `MindMap.macros` round-trips through save+load with absence
    /// preserved (skip_serializing_if = "Vec::is_empty") and
    /// non-empty content preserved exactly. Locks the on-disk
    /// contract for the `macros` field.
    #[test]
    fn test_save_to_file_macros_round_trip() {
        // Empty case: no `macros` key written, no key on reload.
        let blank = MindMap::new_blank("macro-rt");
        assert!(blank.macros.is_empty());
        let tmp_empty = std::env::temp_dir().join("mandala_macros_empty.mindmap.json");
        save_to_file(&tmp_empty, &blank).expect("save failed");
        let reloaded_empty = load_from_file(&tmp_empty).expect("reload failed");
        assert!(reloaded_empty.macros.is_empty());

        // Verify the key is absent on disk (skip_serializing_if).
        let raw = std::fs::read_to_string(&tmp_empty).expect("read raw");
        assert!(!raw.contains("\"macros\""), "empty macros must not be serialised");
        let _ = std::fs::remove_file(&tmp_empty);

        // Non-empty case: round-trip preserves the JSON shape.
        let mut populated = MindMap::new_blank("macro-rt-2");
        populated.macros = vec![serde_json::json!({
            "id": "save-and-quit",
            "name": "Save and Quit",
            "description": "",
            "steps": [{"kind": "Action", "action": "SaveDocument"}]
        })];
        let tmp_full = std::env::temp_dir().join("mandala_macros_full.mindmap.json");
        save_to_file(&tmp_full, &populated).expect("save failed");
        let reloaded_full = load_from_file(&tmp_full).expect("reload failed");
        assert_eq!(reloaded_full.macros.len(), 1);
        assert_eq!(reloaded_full.macros, populated.macros);
        let _ = std::fs::remove_file(&tmp_full);
    }

    /// `MindNode.inline_macros` round-trips through save+load
    /// with absence preserved (skip_serializing_if = "Vec::is_empty")
    /// and non-empty content preserved exactly. Parallel to
    /// `test_save_to_file_macros_round_trip` for the per-node
    /// field added in the Inline-tier macro work.
    #[test]
    fn test_save_to_file_inline_macros_round_trip() {
        // Build a map with one node carrying a populated
        // `inline_macros`. Empty case is implicitly covered by
        // `test_save_blank_map_round_trip` (every node has an
        // empty Vec).
        use crate::mindmap::model::{Canvas, MindNode, MindSection, NodeLayout, NodeStyle, Position, Size};
        use std::collections::HashMap;

        let node = MindNode {
            id: "0".to_string(),
            parent_id: None,
            position: Position { x: 0.0, y: 0.0 },
            size: Size {
                width: 100.0,
                height: 50.0,
            },
            sections: vec![MindSection::new_default("n".to_string(), Vec::new())],
            style: NodeStyle {
                background_color: "#000000".to_string(),
                frame_color: "#ffffff".to_string(),
                text_color: "#ffffff".to_string(),
                shape: "rectangle".to_string(),
                corner_radius_percent: 0.0,
                frame_thickness: 1.0,
                show_frame: true,
                show_shadow: false,
                border: None,
            },
            layout: NodeLayout {
                layout_type: "map".to_string(),
                direction: "auto".to_string(),
                spacing: 0.0,
            },
            folded: false,
            notes: String::new(),
            color_schema: None,
            channel: 0,
            trigger_bindings: Vec::new(),
            inline_mutations: Vec::new(),
            inline_macros: vec![serde_json::json!({
                "id": "0.tag-as-inbox",
                "steps": [{"kind": "Action", "action": "Undo"}]
            })],
            min_zoom_to_render: None,
            max_zoom_to_render: None,
        };
        let mut nodes = HashMap::new();
        nodes.insert("0".to_string(), node);
        let map = MindMap {
            version: "1.0".to_string(),
            name: "inline-rt".to_string(),
            canvas: Canvas {
                background_color: "#000000".to_string(),
                default_border: None,
                default_connection: None,
                default_section_frame_border: None,
                default_focused_section_frame_border: None,
                theme_variables: HashMap::new(),
                theme_variants: HashMap::new(),
            },
            palettes: HashMap::new(),
            nodes,
            edges: Vec::new(),
            custom_mutations: Vec::new(),
            macros: Vec::new(),
        };

        let tmp = std::env::temp_dir().join("mandala_inline_macros_rt.mindmap.json");
        save_to_file(&tmp, &map).expect("save failed");
        let reloaded = load_from_file(&tmp).expect("reload failed");

        let n = reloaded.nodes.get("0").expect("node");
        assert_eq!(n.inline_macros.len(), 1);
        assert_eq!(n.inline_macros[0], map.nodes.get("0").unwrap().inline_macros[0]);

        // Empty-case absence: a node with no inline_macros must
        // not have the key on disk (skip_serializing_if).
        let raw = std::fs::read_to_string(&tmp).expect("read raw");
        // The serialised node has `"inline_macros":` exactly once
        // (the populated one). A second occurrence would mean
        // empty Vecs were serialised. Today the map has exactly
        // one node, so 1 match is correct; if we had a second
        // node with no inline_macros it should be absent.
        assert_eq!(
            raw.matches("\"inline_macros\"").count(),
            1,
            "non-empty inline_macros must be serialised; empty must not"
        );

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_is_hidden_by_fold() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        // Root node has no parent, so it should never be hidden
        let root = map.nodes.get("0").unwrap();
        assert!(!map.is_hidden_by_fold(root));

        // A child of a non-folded parent should not be hidden
        let children = map.children_of("0");
        assert!(!children.is_empty());
        // The root is not folded by default, so its children are visible
        assert!(!map.is_hidden_by_fold(children[0]));
    }
}
