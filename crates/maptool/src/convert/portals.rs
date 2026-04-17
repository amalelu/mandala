//! Legacy `portals` → portal-mode edges migration. Reads a pre-refactor
//! `.mindmap.json` that still has a top-level `portals` array and
//! rewrites each entry as an edge with `display_mode = "portal"`.
//! The loader rejects files that still carry `portals`, so this is
//! the one-way door users with unmigrated maps walk through.
//!
//! Round-trip guarantee: every `PortalPair` field survives the
//! migration —
//! - `endpoint_a` / `endpoint_b` → `from_id` / `to_id`
//! - `glyph` → `glyph_connection.body`
//! - `color` → `edge.color` (the marker reads this when no
//!   `glyph_connection.color` override is set)
//! - `font` / `font_size_pt` → `glyph_connection.{font, font_size_pt}`
//!
//! The portal `label` is dropped — post-refactor portals identify
//! themselves by `(from_id, to_id, edge_type)` like any other edge,
//! so the auto-assigned column-letter label has no role. Users who
//! relied on the label for visual identification were reading the
//! marker glyph, which carries over unchanged.

use serde_json::{json, Value};
use std::path::Path;

/// Read `input_path`, convert any `portals[]` entries into
/// portal-mode edges appended to `edges[]`, and write the result to
/// `output_path`. In-place migrations (input == output) are fine —
/// the read completes before the write begins.
pub fn convert_portals(input_path: &Path, output_path: &Path) -> Result<(), String> {
    let content = std::fs::read_to_string(input_path)
        .map_err(|e| format!("failed to read {}: {e}", input_path.display()))?;

    let mut root: Value = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse {}: {e}", input_path.display()))?;

    let portals = match root.as_object_mut() {
        Some(obj) => obj.remove("portals"),
        None => None,
    };
    let portals_array = match portals {
        Some(Value::Array(a)) => a,
        Some(_) | None => {
            // No portals field, or an unexpected shape: pass through.
            let json = serde_json::to_string_pretty(&root)
                .map_err(|e| format!("failed to serialize: {e}"))?;
            std::fs::write(output_path, &json)
                .map_err(|e| format!("failed to write {}: {e}", output_path.display()))?;
            return Ok(());
        }
    };

    let converted = portals_array.len();
    let edges = root
        .as_object_mut()
        .ok_or("map root must be a JSON object")?
        .entry("edges")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("map `edges` field is not an array")?;

    for portal in portals_array {
        let obj = match portal {
            Value::Object(o) => o,
            _ => continue,
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
        let glyph = obj
            .get("glyph")
            .and_then(|v| v.as_str())
            .unwrap_or("\u{25C8}") // ◈ default
            .to_string();
        let color = obj
            .get("color")
            .and_then(|v| v.as_str())
            .unwrap_or("#aa88cc")
            .to_string();
        let font_size_pt = obj
            .get("font_size_pt")
            .and_then(|v| v.as_f64())
            .unwrap_or(16.0);
        let font = obj.get("font").cloned().unwrap_or(Value::Null);

        let mut glyph_connection = serde_json::Map::new();
        glyph_connection.insert("body".into(), Value::String(glyph));
        glyph_connection.insert(
            "font_size_pt".into(),
            Value::Number(
                serde_json::Number::from_f64(font_size_pt)
                    .unwrap_or_else(|| serde_json::Number::from(16)),
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
    }

    let json = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("failed to serialize: {e}"))?;
    std::fs::write(output_path, &json)
        .map_err(|e| format!("failed to write {}: {e}", output_path.display()))?;

    eprintln!("converted {} portal(s) to portal-mode edges", converted);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn no_portals_field_is_noop() {
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
    fn legacy_portal_becomes_portal_mode_edge() {
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
}
