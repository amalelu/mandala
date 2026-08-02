// SPDX-License-Identifier: MPL-2.0

//! Migrate the legacy top-level `portals` array to edges with
//! `display_mode = "portal"`. Field map:
//!   `endpoint_a`/`endpoint_b` → `from_id`/`to_id`
//!   `glyph`                   → `glyph_connection.body`
//!   `color`                   → `edge.color`
//!   `font`/`font_size_pt`     → `glyph_connection.{font,font_size_pt}`
//! `label` is dropped (post-refactor portals identify by edge tuple).
//!
//! The transform itself is [`fold_portals_into_edges`], which the
//! `--portals` verb and the `--legacy` pipeline both call. A legacy
//! miMind map can carry portals, so folding them is part of the one
//! legacy hop `format/migration.md` promises — not a separate
//! follow-up the user has to know to run.

use serde_json::{json, Value};
use std::path::Path;

/// Fold every entry of the legacy top-level `portals[]` array into a
/// portal-mode edge appended to `edges[]`, removing the `portals` key.
/// Returns the number of edges actually appended.
///
/// That return value is the *only* signal a one-shot migration gives
/// the user, so it counts pushes rather than input length: an entry
/// that is not a JSON object cannot become an edge, and reporting it
/// as folded would turn silent data loss into a plausible-looking
/// success. Each such entry is named on stderr with its index so the
/// drop is diagnosable rather than invisible.
///
/// Idempotent: a tree with no `portals` key (already migrated) folds
/// zero portals and is left byte-identical. A `portals` value that is
/// present but not an array is dropped rather than carried through —
/// no legacy writer produced that shape, and preserving it would
/// keep nothing the loader or the app can read while leaving a key
/// the format no longer defines. (The loader itself only rejects a
/// *non-empty array* `portals`; the other shapes load, they just
/// mean nothing.)
///
/// Errors only on a root that is not a JSON object, or an `edges`
/// key that is not an array — both of which make the file unusable
/// as a mindmap regardless of portals.
pub(super) fn fold_portals_into_edges(root: &mut Value) -> Result<usize, String> {
    let obj = root
        .as_object_mut()
        .ok_or_else(|| "map root must be a JSON object".to_string())?;
    let portals_array = match obj.remove("portals") {
        Some(Value::Array(a)) => a,
        Some(other) => {
            eprintln!(
                "warning: `portals` is {}, not an array; dropped (no legacy writer emits this shape)",
                json_kind(&other)
            );
            return Ok(0);
        }
        None => return Ok(0),
    };

    let edges = obj
        .entry("edges")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("map `edges` field is not an array")?;

    let mut folded = 0usize;
    for (index, portal) in portals_array.into_iter().enumerate() {
        let obj = match portal {
            Value::Object(o) => o,
            other => {
                eprintln!(
                    "warning: portals[{index}] is {}, not an object; dropped",
                    json_kind(&other)
                );
                continue;
            }
        };
        let endpoint_a = obj
            .get("endpoint_a")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let endpoint_b = obj
            .get("endpoint_b")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // An empty `glyph` string would render as a zero-width marker
        // that's impossible to click. Treat `""` the same as missing
        // and fall back to the default marker glyph — a legacy file
        // carrying an empty glyph is almost certainly a bug in
        // whatever tool wrote it.
        let glyph = obj
            .get("glyph")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("\u{25C8}") // ◈ default
            .to_string();
        let color = obj
            .get("color")
            .and_then(|v| v.as_str())
            .unwrap_or("#aa88cc")
            .to_string();
        let font_size_pt = obj.get("font_size_pt").and_then(|v| v.as_f64()).unwrap_or(16.0);
        let font = obj.get("font").cloned().unwrap_or(Value::Null);

        let mut glyph_connection = serde_json::Map::new();
        glyph_connection.insert("body".into(), Value::String(glyph));
        glyph_connection.insert(
            "font_size_pt".into(),
            Value::Number(
                serde_json::Number::from_f64(font_size_pt).unwrap_or_else(|| serde_json::Number::from(16)),
            ),
        );
        if !font.is_null() {
            glyph_connection.insert("font".into(), font);
        }

        let edge = json!({
            "from_id": endpoint_a,
            "to_id": endpoint_b,
            "type": "cross_link",
            "color": color,
            "width": 3,
            "line_style": "solid",
            "visible": true,
            "label": Value::Null,
            "anchor_from": "auto",
            "anchor_to": "auto",
            "control_points": Value::Array(Vec::new()),
            "glyph_connection": Value::Object(glyph_connection),
            "display_mode": "portal",
        });
        edges.push(edge);
        folded += 1;
    }

    Ok(folded)
}

/// Name a JSON value's kind for a warning message. `serde_json` has
/// no public accessor for this, and the value itself may be a whole
/// subtree not worth printing at the user.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Read `input_path`, convert any `portals[]` entries into
/// portal-mode edges appended to `edges[]`, and write the result to
/// `output_path`. In-place migrations (input == output) are fine:
/// the read completes before the write begins, and the write uses
/// a temp-file + rename so a kill mid-write leaves the original
/// intact rather than truncated.
pub fn convert_portals(input_path: &Path, output_path: &Path) -> Result<(), String> {
    super::transform_map_file(input_path, output_path, |root| {
        let folded = fold_portals_into_edges(root)?;
        Ok(format!("converted {folded} portal(s) to portal-mode edges"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_no_portals_field_is_noop() {
        let mut src = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            src,
            r##"{{"version":"1.0","name":"t","canvas":{{"background_color":"#000","default_border":null,"default_connection":null,"theme_variables":{{}},"theme_variants":{{}}}},"nodes":{{}},"edges":[]}}"##
        )
        .unwrap();
        let dst = tempfile::NamedTempFile::new().unwrap();
        convert_portals(src.path(), dst.path()).unwrap();
        let out: Value = serde_json::from_str(&std::fs::read_to_string(dst.path()).unwrap()).unwrap();
        assert!(out.get("portals").is_none());
    }

    #[test]
    fn test_empty_glyph_falls_back_to_default_marker() {
        // A legacy portal with `glyph: ""` would migrate to an edge
        // with an empty `glyph_connection.body`, rendering as a
        // zero-width marker that's impossible to interact with.
        // The converter substitutes the default marker instead.
        let mut src = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            src,
            r##"{{"version":"1.0","name":"t",
              "canvas":{{"background_color":"#000","default_border":null,"default_connection":null,"theme_variables":{{}},"theme_variants":{{}}}},
              "nodes":{{}},"edges":[],
              "portals":[{{"endpoint_a":"0","endpoint_b":"1","label":"A","glyph":"","color":"#ff0","font_size_pt":16.0}}]
            }}"##
        )
        .unwrap();
        let dst = tempfile::NamedTempFile::new().unwrap();
        convert_portals(src.path(), dst.path()).unwrap();
        let out: Value = serde_json::from_str(&std::fs::read_to_string(dst.path()).unwrap()).unwrap();
        let body = &out.get("edges").unwrap().as_array().unwrap()[0]["glyph_connection"]["body"];
        assert_eq!(body.as_str().unwrap(), "\u{25C8}");
    }

    // Atomicity (no tmp leftover after successful rename) is tested
    // canonically in `baumhard::mindmap::loader::tests` against the
    // shared `write_atomic` helper that this migration now uses.

    #[test]
    fn test_legacy_portal_becomes_portal_mode_edge() {
        let mut src = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            src,
            r##"{{"version":"1.0","name":"t",
              "canvas":{{"background_color":"#000","default_border":null,"default_connection":null,"theme_variables":{{}},"theme_variants":{{}}}},
              "nodes":{{}},"edges":[],
              "portals":[{{"endpoint_a":"0","endpoint_b":"1","label":"A","glyph":"⬢","color":"#ff00aa","font_size_pt":20.0}}]
            }}"##
        )
        .unwrap();
        let dst = tempfile::NamedTempFile::new().unwrap();
        convert_portals(src.path(), dst.path()).unwrap();
        let out: Value = serde_json::from_str(&std::fs::read_to_string(dst.path()).unwrap()).unwrap();

        // portals field must be gone.
        assert!(out.get("portals").is_none());
        // edges must have one entry, display_mode=portal, body=⬢.
        let edges = out.get("edges").unwrap().as_array().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["display_mode"], "portal");
        assert_eq!(edges[0]["from_id"], "0");
        assert_eq!(edges[0]["to_id"], "1");
        assert_eq!(edges[0]["color"], "#ff00aa");
        assert_eq!(edges[0]["glyph_connection"]["body"], "⬢");
        assert_eq!(edges[0]["glyph_connection"]["font_size_pt"], 20.0);
    }

    /// `format/migration.md` §"The fold, exactly" prints both halves
    /// of this transform as literal JSON. A doc example that drifts
    /// from the converter is worse than no example — a reader hand-
    /// migrating a map by that block would produce a file the loader
    /// rejects.
    ///
    /// Both blocks are **read out of the spec** rather than restated
    /// here: a hard-copied literal would pin the converter against
    /// the test's own copy and let the doc drift away from both
    /// undetected, which is precisely the failure this test exists to
    /// catch.
    #[test]
    fn test_documented_fold_matches_converter_output() {
        use baumhard::util::doc_fixtures::{documented_json_block, format_doc_path};

        let doc = format_doc_path("migration.md");
        let published_legacy = documented_json_block(&doc, "### The fold, exactly", 0);
        let published_edge = documented_json_block(&doc, "### The fold, exactly", 1);

        let legacy_portal: Value = serde_json::from_str(&published_legacy).unwrap_or_else(|e| {
            panic!("the documented legacy portal block must be valid JSON: {e}\n{published_legacy}")
        });
        let mut root = json!({ "edges": [], "portals": [legacy_portal] });
        assert_eq!(fold_portals_into_edges(&mut root).unwrap(), 1);

        let expected: Value = serde_json::from_str(&published_edge).unwrap_or_else(|e| {
            panic!("the documented portal-edge block must be valid JSON: {e}\n{published_edge}")
        });
        let produced = &root["edges"].as_array().unwrap()[0];
        assert_eq!(
            produced, &expected,
            "converter output drifted from format/migration.md's documented fold"
        );

        // ...and the documented shape is one the typed model accepts,
        // so a reader who copies the block by hand gets a loadable map.
        serde_json::from_str::<baumhard::mindmap::model::MindEdge>(&published_edge)
            .expect("the documented portal edge must deserialize as a MindEdge");
    }

    /// A `portals` key holding something other than an array is
    /// dropped rather than carried through. Note the loader does
    /// *not* reject this shape — it only rejects a non-empty array —
    /// so the reason is that no legacy writer emits it and carrying
    /// it forward would preserve a key the format no longer defines
    /// while conveying nothing the app can read.
    #[test]
    fn test_non_array_portals_key_is_dropped() {
        let mut root = json!({ "edges": [], "portals": "not an array" });
        assert_eq!(fold_portals_into_edges(&mut root).unwrap(), 0);
        assert!(root.get("portals").is_none());
    }

    /// A legacy map with no `edges` key at all still gets its portals:
    /// the fold creates the array rather than silently dropping them.
    #[test]
    fn test_fold_creates_edges_array_when_absent() {
        let mut root = json!({
            "portals": [{ "endpoint_a": "0", "endpoint_b": "1" }]
        });
        assert_eq!(fold_portals_into_edges(&mut root).unwrap(), 1);
        let edges = root["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["display_mode"], "portal");
        // Defaults documented in format/migration.md.
        assert_eq!(edges[0]["glyph_connection"]["body"], "\u{25C8}");
        assert_eq!(edges[0]["color"], "#aa88cc");
        assert_eq!(edges[0]["glyph_connection"]["font_size_pt"], 16.0);
        assert!(edges[0]["glyph_connection"].get("font").is_none());
    }

    /// The returned count is the only signal a one-shot migration
    /// gives the user, so it must equal the edges actually written.
    /// Counting the input array instead reported `3 folded` for an
    /// array holding one usable portal and two junk entries — a
    /// plausible number over silent data loss, with nothing left on
    /// disk for `verify` to notice.
    #[test]
    fn test_count_reflects_edges_written_not_entries_seen() {
        let mut root = json!({
            "edges": [],
            "portals": [
                "oops-a-string",
                { "endpoint_a": "0", "endpoint_b": "1" },
                42
            ]
        });
        let folded = fold_portals_into_edges(&mut root).unwrap();
        let written = root["edges"].as_array().unwrap().len();
        assert_eq!(written, 1, "only the object entry can become an edge");
        assert_eq!(
            folded, written,
            "the reported count must equal the edges actually written"
        );
    }
}
